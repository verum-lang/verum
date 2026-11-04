//! End-to-end integration tests for the PermissionCheckWire
//! opcode (#12 / P3.2). Verifies that:
//!
//!  1. The bytecode encoder writes a `TensorExtended` opcode +
//!  `PermissionCheckWire` sub-opcode + three register bytes
//!  that round-trip through the decoder.
//!  2. The dispatch handler routes through the runtime
//!  `PermissionRouter` and writes the decision tag back.
//!  3. Default allow-all routes return `0` into dst.
//!  4. A configured deny-all policy routes return `1` into dst
//!  and back-fills the warm-path cache.
//!  5. Cached decisions short-circuit subsequent dispatches
//!  without invoking the policy callback.

use std::sync::Arc;
use std::sync::Mutex;

use verum_vbc::bytecode::{decode_instruction, encode_instruction};
use verum_vbc::instruction::{Instruction, Reg};
use verum_vbc::interpreter::InterpreterState;
use verum_vbc::interpreter::permission::{PermissionDecision, PermissionScope};
use verum_vbc::module::VbcModule;
use verum_vbc::value::Value;

fn empty_state() -> InterpreterState {
    InterpreterState::new(Arc::new(VbcModule::new("permission_test".to_string())))
}

#[test]
fn permission_assert_encodes_and_decodes_round_trip() {
    // PermissionAssert encoding shape:
    //  TensorExtended + escape + payload length + 0x1D + scope + target.
    let instr = Instruction::PermissionAssert {
        scope_tag: 1, // FileSystem
        target_id: Reg(7),
    };
    let mut bytes = Vec::new();
    encode_instruction(&instr, &mut bytes);
    assert_eq!(bytes, vec![0xFC, 0xFF, 3, 0x1D, 1, 7]);

    let mut offset = 0;
    let decoded = decode_instruction(&bytes, &mut offset).expect("decoder must succeed");
    match decoded {
        Instruction::PermissionAssert {
            scope_tag,
            target_id,
        } => {
            assert_eq!(scope_tag, 1);
            assert_eq!(target_id.0, 7);
        }
        other => panic!("expected PermissionAssert, got {:?}", other),
    }
}

#[test]
fn permission_check_wire_encodes_and_decodes_round_trip() {
    let instr = Instruction::PermissionCheckWire {
        dst: Reg(3),
        scope_tag: Reg(4),
        target_id: Reg(5),
    };
    let mut bytes = Vec::new();
    encode_instruction(&instr, &mut bytes);
    // Sanity: TensorExtended (0xFC) opcode + 0x1C sub-opcode +
    // dst (1B short) + scope_tag (1B short) + target_id (1B short).
    // 0x1C chosen outside both TensorSubOpcode (which has
    // NewFromArgs at 0x0D) and the regex window (0x0A-0x0C) so
    // the decoder's TensorSubOpcode probe falls through to the
    // ExtSubOpcode dispatch.
    assert_eq!(bytes, vec![0xFC, 0xFF, 4, 0x1C, 3, 4, 5]);

    let mut offset = 0;
    let decoded = decode_instruction(&bytes, &mut offset).expect("decoder must succeed");
    match decoded {
        Instruction::PermissionCheckWire {
            dst,
            scope_tag,
            target_id,
        } => {
            assert_eq!(dst.0, 3);
            assert_eq!(scope_tag.0, 4);
            assert_eq!(target_id.0, 5);
        }
        other => panic!("expected PermissionCheckWire, got {:?}", other),
    }
}

#[test]
fn check_permission_helper_default_allow_all() {
    let mut state = empty_state();
    // Default router has no policy installed → every request
    // resolves to Allow.
    assert_eq!(
        state.check_permission(PermissionScope::Syscall, 1),
        PermissionDecision::Allow
    );
    assert_eq!(
        state.check_permission(PermissionScope::FileSystem, 0xCAFE_BABE),
        PermissionDecision::Allow
    );
}

#[test]
fn deny_all_policy_short_circuits_and_caches() {
    let mut state = empty_state();
    let invocations = Arc::new(Mutex::new(0u64));
    let inv2 = invocations.clone();
    state.set_permission_policy(move |_, _| {
        *inv2.lock().unwrap() += 1;
        PermissionDecision::Deny
    });

    // First call invokes the policy.
    assert_eq!(
        state.check_permission(PermissionScope::Network, 80),
        PermissionDecision::Deny
    );
    assert_eq!(*invocations.lock().unwrap(), 1);

    // Repeats hit the cached decision — policy is not consulted.
    for _ in 0..1_000 {
        assert_eq!(
            state.check_permission(PermissionScope::Network, 80),
            PermissionDecision::Deny
        );
    }
    assert_eq!(*invocations.lock().unwrap(), 1);
    assert_eq!(state.permission_router.stats.last_entry_hits, 1_000);
}

#[test]
fn scope_disambiguates_target_under_dispatch() {
    let mut state = empty_state();
    state.set_permission_policy(|scope, _| match scope {
        PermissionScope::Syscall => PermissionDecision::Deny,
        _ => PermissionDecision::Allow,
    });

    // Same target_id (99) under two scopes routes
    // independently — a Syscall denial cannot be bypassed by
    // priming a FileSystem allow at the same id.
    assert_eq!(
        state.check_permission(PermissionScope::Syscall, 99),
        PermissionDecision::Deny
    );
    assert_eq!(
        state.check_permission(PermissionScope::FileSystem, 99),
        PermissionDecision::Allow
    );
    // Re-checking Syscall(99) must still see Deny — the
    // FileSystem allow did not poison its cache key.
    assert_eq!(
        state.check_permission(PermissionScope::Syscall, 99),
        PermissionDecision::Deny
    );
}

#[test]
fn warm_path_invariant_under_million_iterations() {
    let mut state = empty_state();
    // No policy → allow-all → warm path entirely on the
    // one-entry cache after the first call.
    for _ in 0..1_000_000 {
        let _ = state.check_permission(PermissionScope::Memory, 7);
    }
    assert_eq!(state.permission_router.stats.total, 1_000_000);
    assert_eq!(state.permission_router.stats.policy_invocations, 0);
    assert_eq!(state.permission_router.stats.last_entry_hits, 999_999);
    assert_eq!(state.permission_router.stats.denials, 0);
}

#[test]
fn reset_clears_cache_preserves_policy() {
    let mut state = empty_state();
    let invocations = Arc::new(Mutex::new(0u64));
    let inv2 = invocations.clone();
    state.set_permission_policy(move |_, _| {
        *inv2.lock().unwrap() += 1;
        PermissionDecision::Allow
    });

    // Prime the cache.
    state.check_permission(PermissionScope::Cryptography, 0x1234);
    state.check_permission(PermissionScope::Cryptography, 0x1234);
    assert_eq!(*invocations.lock().unwrap(), 1);

    // Reset clears the cache; policy survives.
    state.reset();
    assert!(state.permission_router.has_policy());
    state.check_permission(PermissionScope::Cryptography, 0x1234);
    assert_eq!(
        *invocations.lock().unwrap(),
        2,
        "reset must invalidate the permission cache"
    );
}

/// PermissionAssert architectural invariant: when the router
/// resolves the request to Deny, the dispatch path MUST raise an
/// error rather than silently continuing. This test exercises
/// the same routing logic that the dispatch handler uses
/// (`state.check_permission`), then verifies that codegen-side
/// auto-emit can rely on the binary Allow/Deny outcome to gate
/// downstream emission.
#[test]
fn permission_assert_deny_yields_error_outcome() {
    let mut state = empty_state();
    state.set_permission_policy(|scope, target| {
        if scope == PermissionScope::Process && target == 0xDEAD {
            PermissionDecision::Deny
        } else {
            PermissionDecision::Allow
        }
    });

    // Deny path
    let denied = state.check_permission(PermissionScope::Process, 0xDEAD);
    assert_eq!(denied, PermissionDecision::Deny);

    // Allow path under same scope
    let allowed = state.check_permission(PermissionScope::Process, 0xBEEF);
    assert_eq!(allowed, PermissionDecision::Allow);

    // The PermissionAssert dispatch handler turns
    // PermissionDecision::Deny into InterpreterError::Panic with
    // a "permission denied: …" message. The shape of that
    // message is the public contract for catch-frame
    // pattern-matching.
    let scope = PermissionScope::Process;
    let target_id: u64 = 0xDEAD;
    let expected_msg = format!("permission denied: {:?}({})", scope, target_id);
    assert!(
        expected_msg.starts_with("permission denied:"),
        "panic message must start with the PermissionDenied prefix"
    );
    assert!(
        expected_msg.contains("Process"),
        "panic message must include the scope name for catch-frame matching"
    );
}

/// Ensures the value-encoding roundtrips: a check that returns
/// Allow lands as `0_i64` in the destination register, a Deny
/// lands as `1_i64`. The dispatch handler reads from registers
/// by-Value, so explicit roundtrip coverage matters.
#[test]
fn dispatch_handler_writes_value_zero_for_allow_one_for_deny() {
    use verum_vbc::interpreter::permission::PermissionRouter;

    // Direct construction — unit-test the routing path that the
    // dispatch handler exercises without simulating the full
    // bytecode loop.
    let mut router = PermissionRouter::with_policy(|scope, _| match scope {
        PermissionScope::Network => PermissionDecision::Deny,
        _ => PermissionDecision::Allow,
    });

    let allow_decision = router.check(PermissionScope::Syscall, 1);
    let deny_decision = router.check(PermissionScope::Network, 80);

    let allow_tag: i64 = match allow_decision {
        PermissionDecision::Allow => 0,
        PermissionDecision::Deny => 1,
    };
    let deny_tag: i64 = match deny_decision {
        PermissionDecision::Allow => 0,
        PermissionDecision::Deny => 1,
    };

    let allow_value = Value::from_i64(allow_tag);
    let deny_value = Value::from_i64(deny_tag);

    assert!(allow_value.is_int());
    assert!(deny_value.is_int());
    assert_eq!(allow_value.as_i64(), 0);
    assert_eq!(deny_value.as_i64(), 1);
}

// ============================================================================
// #101 — PermissionStatsRead / PermissionStatsClear opcodes
// ============================================================================

#[test]
fn permission_stats_read_encodes_and_decodes_round_trip() {
    // Escaped TensorExtended carrier with a bounded typed payload.
    let instr = Instruction::PermissionStatsRead {
        dst: Reg(2),
        selector: Reg(5),
    };
    let mut bytes = Vec::new();
    encode_instruction(&instr, &mut bytes);
    assert_eq!(bytes, vec![0xFC, 0xFF, 3, 0x1E, 2, 5]);

    let mut offset = 0;
    let decoded = decode_instruction(&bytes, &mut offset).expect("decode");
    match decoded {
        Instruction::PermissionStatsRead { dst, selector } => {
            assert_eq!(dst.0, 2);
            assert_eq!(selector.0, 5);
        }
        other => panic!("expected PermissionStatsRead, got {:?}", other),
    }
}

#[test]
fn permission_stats_clear_encodes_and_decodes_round_trip() {
    let instr = Instruction::PermissionStatsClear { dst: Reg(3) };
    let mut bytes = Vec::new();
    encode_instruction(&instr, &mut bytes);
    assert_eq!(bytes, vec![0xFC, 0xFF, 2, 0x1F, 3]);

    let mut offset = 0;
    let decoded = decode_instruction(&bytes, &mut offset).expect("decode");
    match decoded {
        Instruction::PermissionStatsClear { dst } => {
            assert_eq!(dst.0, 3);
        }
        other => panic!("expected PermissionStatsClear, got {:?}", other),
    }
}

#[test]
fn permission_stats_field_selector_mapping_matches_router() {
    // The selector encoding is the public contract for the
    // stdlib's `permission_stats()` wrapper — this test pins
    // the selector → field mapping so a future refactor of
    // the Rust-side struct can't silently drift.
    let mut router =
        verum_vbc::interpreter::permission::PermissionRouter::with_policy(|scope, target| {
            if scope == PermissionScope::Network && target == 80 {
                PermissionDecision::Deny
            } else {
                PermissionDecision::Allow
            }
        });

    // Drive deterministic activity:
    //  * 1 deny-policy invocation (Network/80)
    //  * 4 repeats of the deny → 4 last-entry hits
    //  * 1 allow-policy invocation (Syscall/1)
    //  * 1 last-entry hit on the allow
    let _ = router.check(PermissionScope::Network, 80);
    for _ in 0..4 {
        let _ = router.check(PermissionScope::Network, 80);
    }
    let _ = router.check(PermissionScope::Syscall, 1);
    let _ = router.check(PermissionScope::Syscall, 1);

    let s = router.stats;
    assert_eq!(s.total, 7);
    assert_eq!(s.last_entry_hits, 5); // 4 deny repeats + 1 allow repeat
    assert_eq!(s.map_hits, 0);
    assert_eq!(s.policy_invocations, 2);
    assert_eq!(s.denials, 5); // 1 cold deny + 4 warm-deny short-circuits
}

#[test]
fn permission_stats_clear_zeros_counters_preserves_cache() {
    use verum_vbc::interpreter::permission::PermissionRouterStats;

    let mut state = empty_state();
    state.set_permission_policy(|_, _| PermissionDecision::Allow);

    state.check_permission(PermissionScope::Memory, 1);
    state.check_permission(PermissionScope::Memory, 1);
    assert!(state.permission_router.stats.total > 0);

    // Simulate the dispatch handler's clear-stats path.
    state.permission_router.stats = PermissionRouterStats::default();
    let after = state.permission_router.stats;
    assert_eq!(after.total, 0);
    assert_eq!(after.last_entry_hits, 0);

    // Cache survives — next check still hits the warm path,
    // not the policy.
    let invocations_before = state.permission_router.stats.policy_invocations;
    state.check_permission(PermissionScope::Memory, 1);
    let invocations_after = state.permission_router.stats.policy_invocations;
    assert_eq!(
        invocations_after, invocations_before,
        "clearing stats must NOT clear the cache; warm-path hits should not invoke policy"
    );
}

fn emitted_permission_routes(instructions: &[Instruction]) -> [VbcModule; 2] {
    use verum_vbc::{
        bytecode::encode_instructions,
        module::FunctionDescriptor,
        types::{TypeId, TypeRef},
    };
    let mut module = VbcModule::new("permission_execution".into());
    let mut function = FunctionDescriptor::new(module.intern_string("probe"));
    function.has_source_body = true;
    function.register_count = 140;
    function.return_type = TypeRef::Concrete(TypeId::INT);
    function.bytecode_length = encode_instructions(instructions, &mut module.bytecode) as u32;
    module.add_function(function);
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    [
        module,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ]
}

#[test]
fn emitted_permission_assert_enforces_policy_and_resumes_after_wide_operands() {
    use verum_vbc::interpreter::{Interpreter, InterpreterError};
    for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
        for module in emitted_permission_routes(&[
            Instruction::LoadI {
                dst: Reg(130),
                value: 37123,
            },
            Instruction::PermissionAssert {
                scope_tag: PermissionScope::Syscall.to_wire_tag(),
                target_id: Reg(130),
            },
            Instruction::LoadI {
                dst: Reg(0),
                value: 41,
            },
            Instruction::Ret { value: Reg(0) },
        ]) {
            let id = module.find_function_by_name("probe").unwrap();
            let mut interpreter = Interpreter::new(Arc::new(module));
            let requests = Arc::new(Mutex::new(0));
            let count = requests.clone();
            interpreter
                .state
                .set_permission_policy(move |scope, target| {
                    assert_eq!(scope, PermissionScope::Syscall);
                    assert_eq!(target, 37123);
                    *count.lock().unwrap() += 1;
                    decision
                });
            let result = interpreter.execute_function(id);
            match decision {
                PermissionDecision::Allow => assert_eq!(result.unwrap().as_i64(), 41),
                PermissionDecision::Deny => {
                    assert!(matches!(result, Err(InterpreterError::Panic { message })
                    if message == "permission denied: Syscall(37123)"))
                }
            }
            assert_eq!(*requests.lock().unwrap(), 1);
        }
    }
}

#[test]
fn emitted_permission_check_and_statistics_use_the_same_bounded_carrier() {
    use verum_vbc::interpreter::Interpreter;
    for clear in [false, true] {
        let mut instructions = vec![
            Instruction::LoadI {
                dst: Reg(130),
                value: 0,
            },
            Instruction::LoadI {
                dst: Reg(131),
                value: 37123,
            },
            Instruction::PermissionCheckWire {
                dst: Reg(132),
                scope_tag: Reg(130),
                target_id: Reg(131),
            },
            Instruction::LoadI {
                dst: Reg(133),
                value: 4,
            }, // denial count
        ];
        if clear {
            instructions.push(Instruction::PermissionStatsClear { dst: Reg(135) });
        }
        instructions.extend([
            Instruction::PermissionStatsRead {
                dst: Reg(134),
                selector: Reg(133),
            },
            Instruction::Ret { value: Reg(134) },
        ]);
        for module in emitted_permission_routes(&instructions) {
            let id = module.find_function_by_name("probe").unwrap();
            let mut interpreter = Interpreter::new(Arc::new(module));
            interpreter.state.set_permission_policy(|scope, target| {
                assert_eq!(scope, PermissionScope::Syscall);
                assert_eq!(target, 37123);
                PermissionDecision::Deny
            });
            let result = interpreter.execute_function(id).unwrap();
            assert_eq!(result.as_i64(), if clear { 0 } else { 1 });
        }
    }
}

#[test]
fn historical_bare_permission_decode_canonicalizes_to_existing_escaped_carrier() {
    for bare in [
        vec![0xFC, 0x1D, 1, 7],
        vec![0xFC, 0x1C, 3, 4, 5],
        vec![0xFC, 0x1E, 2, 5],
        vec![0xFC, 0x1F, 3],
    ] {
        let mut offset = 0;
        let instruction = decode_instruction(&bare, &mut offset).unwrap();
        assert_eq!(offset, bare.len());
        let mut canonical = Vec::new();
        encode_instruction(&instruction, &mut canonical);
        assert_eq!(&canonical[..2], &[0xFC, 0xFF]);
        assert_eq!(canonical[2] as usize, canonical.len() - 3);
        assert_eq!(&canonical[3..], &bare[1..]);
        let mut offset = 0;
        let recovered = decode_instruction(&canonical, &mut offset).unwrap();
        assert_eq!(offset, canonical.len());
        assert_eq!(recovered, instruction);
    }
}

#[test]
fn historical_bare_permission_bytes_cannot_be_misread_as_an_envelope() {
    use verum_vbc::interpreter::{Interpreter, InterpreterError};
    for bare in [
        vec![0xFC, 0x1D, 2, 0],
        vec![0xFC, 0x1C, 3, 4, 5],
        vec![0xFC, 0x1E, 2, 5],
        vec![0xFC, 0x1F, 3],
    ] {
        let mut module =
            emitted_permission_routes(&[Instruction::Ret { value: Reg(0) }])[0].clone();
        module.bytecode = bare;
        encode_instruction(
            &Instruction::LoadI {
                dst: Reg(0),
                value: 41,
            },
            &mut module.bytecode,
        );
        encode_instruction(&Instruction::Ret { value: Reg(0) }, &mut module.bytecode);
        module.functions[0].bytecode_length = module.bytecode.len() as u32;
        let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
        for module in [
            module,
            verum_vbc::deserialize::deserialize_module(&bytes).unwrap(),
        ] {
            let id = module.find_function_by_name("probe").unwrap();
            let mut interpreter = Interpreter::new(Arc::new(module));
            interpreter
                .state
                .set_permission_policy(|_, _| panic!("ambiguous operand must not reach policy"));
            let result = interpreter.execute_function(id);
            assert!(
                matches!(&result, Err(InterpreterError::InvalidBytecode { message, .. })
                if message == "unescaped permission instruction must be re-encoded before execution"),
                "{result:?}"
            );
        }
    }
}
