//! **T0144 — CROSS-MODULE-CALL identity pins.**
//!
//! The campaign invariant: a cross-module call has exactly ONE
//! resolution authority — the carried-fact band map
//! (`VbcModule::resolve_external_bands` → `resolve_band_id`) — and a
//! reference that authority cannot resolve dies LOUD **naming the
//! qualified callee**.  It must never silently become "whatever
//! function occupies the raw number" (the
//! `Deque.reallocate → AdjacencyList.add_edge` misroute class), and it
//! must never surface as a bare sentinel integer the reader cannot map
//! back to a call site.
//!
//! Pinned here:
//!  1. the authority RESOLVES a band reference to the concrete body,
//!     and `resolved_function_id` is the single normalisation entry
//!     point every consumer goes through;
//!  2. a deliberately-ORPHANED band call (recorded name, no body) dies
//!     with a diagnostic that CONTAINS the qualified callee name;
//!  3. the same orphan stays lenient-skippable inside a `__tls_init_*`
//!     ctor (the CTOR-UNWIND contract that keeps one unresolved static
//!     from aborting an entire test file).

use std::sync::Arc;
use verum_vbc::bytecode;
use verum_vbc::instruction::{Instruction, Reg, RegRange};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule, XMOD_CALL_ID_BAND_BASE};

/// The qualified spelling the precompile records for the cross-module
/// callee — the string the diagnostic must surface.
const ORPHAN_CALLEE: &str = "core.collections.pruned.orphan_callee";

fn encode(instructions: &[Instruction]) -> Vec<u8> {
    let mut bc = Vec::new();
    for instr in instructions {
        bytecode::encode_instruction(instr, &mut bc);
    }
    bc
}

/// `main` calls a band id recorded in `external_function_names` under
/// [`ORPHAN_CALLEE`].  `with_body` decides whether a real body for that
/// name exists in the table — i.e. whether the authority can resolve it.
fn build_module(with_body: bool) -> VbcModule {
    let mut module = VbcModule::new("xmod_band_resolution_pins".to_string());
    let band_id = XMOD_CALL_ID_BAND_BASE + 6;

    let main_body = encode(&[
        Instruction::Call {
            dst: Reg(0),
            func_id: band_id,
            args: RegRange {
                start: Reg(0),
                count: 0,
            },
        },
        Instruction::Ret { value: Reg(0) },
    ]);
    // The resolvable target: `LoadSmallI 42; Ret`.
    let callee_body = encode(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 42,
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

    let mut bc = main_body.clone();
    if with_body {
        let callee_name = module.intern_string(ORPHAN_CALLEE);
        let mut callee_desc = FunctionDescriptor::new(callee_name);
        callee_desc.id = FunctionId(1);
        callee_desc.bytecode_offset = main_body.len() as u32;
        callee_desc.bytecode_length = callee_body.len() as u32;
        callee_desc.register_count = 8;
        module.functions.push(callee_desc);
        bc.extend_from_slice(&callee_body);
    }
    module.bytecode = bc;

    // The precompile's authoritative record: THIS band id, in THIS
    // module's bodies, is a cross-module call to ORPHAN_CALLEE.
    let name_sid = module.intern_string(ORPHAN_CALLEE);
    module
        .external_function_names
        .push((FunctionId(band_id), name_sid));
    module
}

#[test]
fn band_reference_resolves_through_the_one_authority() {
    let mut module = build_module(true);
    let unresolved = module.resolve_external_bands();
    assert!(
        unresolved.is_empty(),
        "a band id whose named body IS in the table must resolve; unresolved={:?}",
        unresolved
    );

    let band_id = XMOD_CALL_ID_BAND_BASE + 6;
    assert_eq!(
        module.resolve_band_id(band_id),
        Some(FunctionId(1)),
        "the authority must map the band id onto the concrete body"
    );
    // The ONE normalisation entry point every call-resolving consumer
    // uses: band ids translate, ordinary ids pass through untouched.
    assert_eq!(module.resolved_function_id(band_id), FunctionId(1));
    assert_eq!(module.resolved_function_id(0), FunctionId(0));

    // ...and dispatch actually reaches the body (no misroute).
    let mut interp = Interpreter::new(Arc::new(module));
    let v = interp
        .execute_function(FunctionId(0))
        .expect("resolved band call must dispatch to the recorded callee");
    assert_eq!(v.as_i64(), 42);
}

#[test]
fn orphaned_band_call_dies_loud_naming_the_qualified_callee() {
    let mut module = build_module(false);
    let unresolved = module.resolve_external_bands();
    assert_eq!(
        unresolved.len(),
        1,
        "an orphaned band reference must be REPORTED unresolved, not silently dropped"
    );
    assert_eq!(unresolved[0].1, ORPHAN_CALLEE);

    let mut interp = Interpreter::new(Arc::new(module));
    let err = interp
        .execute_function(FunctionId(0))
        .expect_err("an orphaned band call must never dispatch to an arbitrary function");
    let rendered = format!("{}", err);
    assert!(
        rendered.contains(ORPHAN_CALLEE),
        "the diagnostic must NAME the qualified callee (never a bare sentinel id); got: {}",
        rendered
    );
}

#[test]
fn orphaned_band_call_in_tls_ctor_stays_lenient() {
    // CTOR-UNWIND contract: naming the callee must not turn a crashed
    // `__tls_init_*` ctor into a fatal error — one unresolved static
    // may not abort every test in the file.
    let mut module = build_module(false);
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
    let ctor_name = module.intern_string("__tls_init_PINNED");
    let mut ctor_desc = FunctionDescriptor::new(ctor_name);
    ctor_desc.id = FunctionId(module.functions.len() as u32);
    ctor_desc.bytecode_offset = module.bytecode.len() as u32;
    ctor_desc.bytecode_length = ctor_body.len() as u32;
    ctor_desc.register_count = 8;
    let ctor_id = ctor_desc.id;
    module.functions.push(ctor_desc);
    module.bytecode.extend_from_slice(&ctor_body);
    module.global_ctors.push((ctor_id, 65535));
    let _ = module.resolve_external_bands();

    let mut interp = Interpreter::new(Arc::new(module));
    interp
        .run_global_ctors()
        .expect("an unresolved band call inside a TLS ctor must be lenient-skipped");
    assert_eq!(
        interp.state.call_stack.depth(),
        0,
        "the lenient skip must unwind the crashed ctor's frames"
    );
}
