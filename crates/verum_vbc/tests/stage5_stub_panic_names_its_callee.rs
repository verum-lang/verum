//! **T1172 — an unresolved stage-5 stub must NAME the callee.**
//!
//! A stage-N stub id that reaches dispatch with no body is a build
//! defect, and the panic is the only place a reader learns which one.
//! Until this pin it printed a bare sentinel number plus advice
//! ("check for `[lenient] SKIP <Type>.<method>` warnings") that, for
//! this class, points at a log with nothing in it — two sessions spent
//! an hour each walking back from the number by hand.
//!
//! The name is not new information the panic has to invent: the module
//! records the qualified callee of every band/stub reference in
//! `external_function_names`, and `band_reference_name` already reads
//! it for the XMOD branch a few lines away in the same handler.
//!
//! BOTH POLARITIES, because a test that only asserts the name is there
//! cannot tell a working lookup from a hardcoded string:
//!
//!  * entry present → the message CONTAINS the qualified name;
//!  * entry absent  → the message is still the stage-5 panic, still
//!    carries the id, and does NOT invent a name.

use std::sync::Arc;
use verum_vbc::bytecode;
use verum_vbc::instruction::{Instruction, Reg, RegRange};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
use verum_vbc::stub_ranges::STAGE5_BASE;

/// The qualified spelling a stage-5 mount-miss stub records.
const STUB_CALLEE: &str = "core.sys.pruned.stage5_callee";

/// Deliberately NOT the base itself — the base is the id every context
/// used to hand out first (the collision this task's sibling closed),
/// so pinning on it would leave the test passing for the wrong reason.
fn stub_id() -> u32 {
    STAGE5_BASE - 7
}

fn encode(instructions: &[Instruction]) -> Vec<u8> {
    let mut bc = Vec::new();
    for instr in instructions {
        bytecode::encode_instruction(instr, &mut bc);
    }
    bc
}

/// `main` calls a stage-5 stub id that has no body anywhere.
/// `with_name` decides whether the module recorded what it was a call TO.
fn build_module(with_name: bool) -> VbcModule {
    let mut module = VbcModule::new("stage5_stub_panic_names_its_callee".to_string());

    let main_body = encode(&[
        Instruction::Call {
            dst: Reg(0),
            func_id: stub_id(),
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
    module.bytecode = main_body;

    if with_name {
        let sid = module.intern_string(STUB_CALLEE);
        module
            .external_function_names
            .push((FunctionId(stub_id()), sid));
    }
    module
}

#[test]
fn an_unresolved_stage5_stub_names_the_callee_it_could_not_reach() {
    let module = build_module(true);
    let mut interp = Interpreter::new(Arc::new(module));
    let err = interp
        .execute_function(FunctionId(0))
        .expect_err("a stage-5 stub with no body must never dispatch");
    let rendered = format!("{}", err);
    assert!(
        rendered.contains(STUB_CALLEE),
        "the stage-5 panic must NAME the callee the module recorded; got: {}",
        rendered
    );
    assert!(
        rendered.contains(&stub_id().to_string()),
        "naming the callee must not cost the id — a reader needs both; got: {}",
        rendered
    );
}

#[test]
fn a_stage5_stub_with_no_recorded_name_says_so_by_omission() {
    // The control that makes the test above mean something: same id,
    // same panic, no entry in `external_function_names`.  If this one
    // also carried the name, the name would be coming from somewhere
    // other than the lookup under test.
    let module = build_module(false);
    let mut interp = Interpreter::new(Arc::new(module));
    let err = interp
        .execute_function(FunctionId(0))
        .expect_err("a stage-5 stub with no body must never dispatch");
    let rendered = format!("{}", err);
    assert!(
        !rendered.contains(STUB_CALLEE),
        "with no recorded name the panic must not produce one; got: {}",
        rendered
    );
    assert!(
        rendered.contains("stage-5"),
        "the panic must still identify the stage; got: {}",
        rendered
    );
    assert!(
        rendered.contains(&stub_id().to_string()),
        "the panic must still carry the id; got: {}",
        rendered
    );
}
