//! CATCH-UNWIND-CLOSURE-1 / PANIC-CLASS-1 pins (T0148).
//!
//! Root defect pinned here: the Tier-0 `catch_unwind` intercept
//! (`interpreter/dispatch_table/handlers/panic_runtime.rs`) accepted only a
//! bare `FuncRef` argument. Since #110 the codegen emits `NewClosure` heap
//! objects for EVERY lambda and named-fn reference — including zero-capture
//! ones — so the intercept never fired on real code: every panic escaped
//! `catch_unwind`, and the whole `assert_panics` / `catch_unwind`
//! conformance surface was red (base/panic: 20+ tests, plus the panic
//! escaping killed sibling tests in the same batch).
//!
//! Second leg: the intercept caught only `InterpreterError::Panic`, but the
//! panic SURFACE lowers to three distinct interpreter errors — `Panic`
//! (`Instruction::Panic`), `AssertionFailed` (`Instruction::Assert`, the
//! builtin `assert`) and `Unreachable` (`Instruction::Unreachable`, the
//! builtin `unreachable()`). All three are panics in language terms and
//! MUST be catchable; runtime faults and `ProcessExit` must NOT be.
//!
//! The pins drive the real dispatch path (`Instruction::Call` into the
//! intercept) with each callable shape and each panic class.

use std::sync::Arc;
use verum_vbc::bytecode;
use verum_vbc::instruction::{Instruction, Reg, RegRange};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::types::TypeId;

fn encode(instructions: &[Instruction]) -> Vec<u8> {
    let mut bc = Vec::new();
    for instr in instructions {
        bytecode::encode_instruction(instr, &mut bc);
    }
    bc
}

/// How the callee is handed to `catch_unwind`.
///
/// Only the heap-closure shape is expressible in bytecode: there is no
/// `LoadFuncRef` instruction — `NewClosure` is what codegen emits for
/// every lambda and named-fn reference since #110, which is precisely
/// why the pre-fix `is_func_ref()`-only gate never fired. The FuncRef
/// arm of the intercept is exercised directly in
/// `func_ref_callable_still_accepted` below via `Value::from_function`.
#[derive(Clone, Copy, PartialEq)]
enum Callable {
    /// `NewClosure` heap object — what codegen actually emits (#110).
    HeapClosure,
}

/// What the callee body does.
#[derive(Clone, Copy)]
enum Body {
    Panic,
    AssertFalse,
    Unreachable,
    ReturnInt(i64),
}

/// Builds a module with:
///  * fn 0 `main` — materialises the callable, then
///    `Call catch_unwind(callable)`, returning its result;
///  * fn 1 `catch_unwind` — an EMPTY body (bytecode_length 0). The name is
///    what the intercept matches on; an empty body means a miss would
///    surface as a Unit return rather than silently doing something else.
///  * fn 2 `victim` — the callee.
fn build_module(callable: Callable, body: Body) -> Arc<VbcModule> {
    let mut module = VbcModule::new("catch_unwind_closure_pin".to_string());

    // main: r1 = <callable>; r0 = catch_unwind(r1); ret r0
    let materialise = match callable {
        Callable::HeapClosure => Instruction::NewClosure {
            dst: Reg(1),
            func_id: 2,
            captures: vec![],
        },
    };
    let main_body = encode(&[
        materialise,
        Instruction::Call {
            dst: Reg(0),
            func_id: 1,
            args: RegRange {
                start: Reg(1),
                count: 1,
            },
        },
        Instruction::Ret { value: Reg(0) },
    ]);

    let victim_body = encode(&match body {
        Body::Panic => vec![Instruction::Panic { message_id: 0 }],
        Body::AssertFalse => vec![
            Instruction::LoadFalse { dst: Reg(0) },
            Instruction::Assert {
                cond: Reg(0),
                message_id: 0,
            },
            Instruction::Ret { value: Reg(0) },
        ],
        Body::Unreachable => vec![Instruction::Unreachable],
        Body::ReturnInt(v) => vec![
            Instruction::LoadSmallI {
                dst: Reg(0),
                value: v as i8,
            },
            Instruction::Ret { value: Reg(0) },
        ],
    });

    let main_name = module.intern_string("main");
    let mut main_desc = FunctionDescriptor::new(main_name);
    main_desc.id = FunctionId(0);
    main_desc.bytecode_offset = 0;
    main_desc.bytecode_length = main_body.len() as u32;
    main_desc.register_count = 8;
    module.functions.push(main_desc);

    // The intercept keys on the callee's NAME; body stays empty.
    let cu_name = module.intern_string("core.base.panic.catch_unwind");
    let mut cu_desc = FunctionDescriptor::new(cu_name);
    cu_desc.id = FunctionId(1);
    cu_desc.bytecode_offset = main_body.len() as u32;
    cu_desc.bytecode_length = 0;
    cu_desc.register_count = 8;
    module.functions.push(cu_desc);

    let victim_name = module.intern_string("victim");
    let mut victim_desc = FunctionDescriptor::new(victim_name);
    victim_desc.id = FunctionId(2);
    victim_desc.bytecode_offset = main_body.len() as u32;
    victim_desc.bytecode_length = victim_body.len() as u32;
    victim_desc.register_count = 8;
    module.functions.push(victim_desc);

    let mut bc = main_body;
    bc.extend_from_slice(&victim_body);
    module.bytecode = bc;

    Arc::new(module)
}

/// Variant of [`build_module`] whose `main` forwards its FIRST ARGUMENT
/// (register 0) to `catch_unwind` — lets a test seed a callable shape
/// that no bytecode instruction can materialise (a bare `FuncRef`).
/// The panicking callee is fn 2, as in `build_module`.
fn build_forwarding_module() -> Arc<VbcModule> {
    let mut module = VbcModule::new("catch_unwind_funcref_pin".to_string());

    // main(callable) : r1 = catch_unwind(r0); ret r1
    let main_body = encode(&[
        Instruction::Call {
            dst: Reg(1),
            func_id: 1,
            args: RegRange {
                start: Reg(0),
                count: 1,
            },
        },
        Instruction::Ret { value: Reg(1) },
    ]);
    let victim_body = encode(&[Instruction::Panic { message_id: 0 }]);

    let main_name = module.intern_string("main");
    let mut main_desc = FunctionDescriptor::new(main_name);
    main_desc.id = FunctionId(0);
    main_desc.bytecode_offset = 0;
    main_desc.bytecode_length = main_body.len() as u32;
    main_desc.register_count = 8;
    module.functions.push(main_desc);

    let cu_name = module.intern_string("core.base.panic.catch_unwind");
    let mut cu_desc = FunctionDescriptor::new(cu_name);
    cu_desc.id = FunctionId(1);
    cu_desc.bytecode_offset = main_body.len() as u32;
    cu_desc.bytecode_length = 0;
    cu_desc.register_count = 8;
    module.functions.push(cu_desc);

    let victim_name = module.intern_string("victim");
    let mut victim_desc = FunctionDescriptor::new(victim_name);
    victim_desc.id = FunctionId(2);
    victim_desc.bytecode_offset = main_body.len() as u32;
    victim_desc.bytecode_length = victim_body.len() as u32;
    victim_desc.register_count = 8;
    module.functions.push(victim_desc);

    let mut bc = main_body;
    bc.extend_from_slice(&victim_body);
    module.bytecode = bc;

    Arc::new(module)
}

/// `Result` variant tag of a heap variant value: `Ok = 0`, `Err = 1`.
fn result_tag(interp: &Interpreter, v: verum_vbc::value::Value) -> u32 {
    assert!(v.is_ptr() && !v.is_nil(), "catch_unwind must return a heap Result variant, got {v:?}");
    let _ = interp;
    // SAFETY: verified non-nil heap pointer produced by `wrap_in_variant`.
    unsafe { verum_vbc::interpreter::variant_tag(v.as_ptr::<u8>()) }
}

#[test]
fn catch_unwind_catches_panic_from_heap_closure() {
    // THE regression: codegen emits NewClosure for every lambda, so this
    // is the shape real `catch_unwind(|| panic("..."))` code produces.
    // Pre-fix the intercept declined (not a FuncRef) and the panic
    // escaped as an interpreter error.
    let mut interp = Interpreter::new(build_module(Callable::HeapClosure, Body::Panic));
    let v = interp
        .execute_function(FunctionId(0))
        .expect("a panic inside catch_unwind must be caught, not propagated");
    assert_eq!(result_tag(&interp, v), 1, "panic must yield Result.Err");
}

#[test]
fn func_ref_callable_still_accepted() {
    // The historical zero-capture FuncRef shape must keep working — the
    // fix WIDENS the accepted set, it does not swap one shape for
    // another. No bytecode instruction materialises a bare FuncRef, so
    // the callable is seeded as an ARGUMENT (register 0) and passed
    // straight through to catch_unwind.
    use verum_vbc::value::Value;
    let mut interp = Interpreter::new(build_forwarding_module());
    let func_ref = Value::from_function(FunctionId(2));
    assert!(func_ref.is_func_ref(), "sanity: FuncRef value shape");
    let v = interp
        .execute_function_with_args(FunctionId(0), &[func_ref])
        .expect("a FuncRef callee must still be accepted by the intercept");
    assert_eq!(result_tag(&interp, v), 1, "panic must yield Result.Err");
}

#[test]
fn catch_unwind_catches_builtin_assert_failure() {
    // PANIC-CLASS-1: builtin `assert(false)` raises AssertionFailed, not
    // Panic. `assert_panics(|| assert(false, ...))` is the single most
    // common shape in the conformance suite.
    let mut interp = Interpreter::new(build_module(Callable::HeapClosure, Body::AssertFalse));
    let v = interp
        .execute_function(FunctionId(0))
        .expect("a failed builtin assert is a panic and must be catchable");
    assert_eq!(result_tag(&interp, v), 1, "assert failure must yield Result.Err");
}

#[test]
fn catch_unwind_catches_builtin_unreachable() {
    // PANIC-CLASS-1: builtin `unreachable()` raises Unreachable.
    let mut interp = Interpreter::new(build_module(Callable::HeapClosure, Body::Unreachable));
    let v = interp
        .execute_function(FunctionId(0))
        .expect("unreachable() is a panic and must be catchable");
    assert_eq!(result_tag(&interp, v), 1, "unreachable must yield Result.Err");
}

#[test]
fn catch_unwind_passes_through_normal_return() {
    // T0619 regression: a NORMAL-returning closure must yield Result.Ok and
    // NOT run away. Pre-fix, `catch_unwind` ran the closure via
    // `execute_table_with_args` (the top-level entry primitive, entry frame
    // `return_pc = 0`); a nested normal `Ret` clobbered `main`'s pc to 0 and
    // spun to InstructionLimitExceeded. The fix routes through
    // `call_function_sync` (the nested primitive that preserves caller pc/r0).
    // The happy path must stay Ok — and must not be confused for a panic.
    let mut interp = Interpreter::new(build_module(Callable::HeapClosure, Body::ReturnInt(42)));
    let v = interp
        .execute_function(FunctionId(0))
        .expect("a normal return must not be treated as a panic");
    assert_eq!(result_tag(&interp, v), 0, "normal return must yield Result.Ok");
}

#[test]
fn catch_unwind_unwinds_frames_on_caught_panic() {
    // CTOR-UNWIND discipline: a caught panic must leave the call stack at
    // its pre-call depth. A leaked frame makes the NEXT execution Ret-pop
    // into a dead frame and re-raise the caught error as its own.
    let mut interp = Interpreter::new(build_module(Callable::HeapClosure, Body::Panic));
    let _ = interp
        .execute_function(FunctionId(0))
        .expect("panic caught");
    assert_eq!(
        interp.state.call_stack.depth(),
        0,
        "catching a panic must unwind the callee's frames"
    );
    // And the interpreter stays usable for the next test body.
    let v = interp
        .execute_function(FunctionId(0))
        .expect("a second execution must not inherit the caught panic");
    assert_eq!(result_tag(&interp, v), 1);
}

#[test]
fn non_closure_heap_argument_is_declined_not_misread() {
    // Loud-over-silent: a non-closure heap object (wrong TypeId) must NOT
    // be misread as a closure. The intercept declines; the call then
    // reaches catch_unwind's own (empty) body instead of executing
    // whatever function id the object's bytes happened to spell.
    let mut module = VbcModule::new("catch_unwind_non_closure".to_string());
    let main_body = encode(&[
        // Allocate a plain record object, NOT a closure (TypeId != 0xC000).
        Instruction::New {
            dst: Reg(1),
            type_id: TypeId(0x1234).0,
            field_count: 1,
        },
        Instruction::Call {
            dst: Reg(0),
            func_id: 1,
            args: RegRange {
                start: Reg(1),
                count: 1,
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
    let cu_name = module.intern_string("core.base.panic.catch_unwind");
    let mut cu_desc = FunctionDescriptor::new(cu_name);
    cu_desc.id = FunctionId(1);
    cu_desc.bytecode_offset = main_body.len() as u32;
    cu_desc.bytecode_length = 0;
    cu_desc.register_count = 8;
    module.functions.push(cu_desc);
    module.bytecode = main_body;

    let mut interp = Interpreter::new(Arc::new(module));
    // Whatever the empty-body callee returns, the point is that we did NOT
    // execute an arbitrary function id decoded from foreign object bytes.
    let _ = interp.execute_function(FunctionId(0));
    assert_eq!(
        interp.state.call_stack.depth(),
        0,
        "declining must leave the stack clean"
    );
}
