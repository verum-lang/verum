//! T0184 — `@vbc(SAMPLE_TOP_K, …)` reaches the MlExtended (0xFD) carrier.
//!
//! The 0xFD carrier already had a dispatch handler, a decode arm and wire
//! pins, and the expression codegen already emitted it — but only from the
//! `verum_*` extern-name path, never from an intrinsic. `CodegenStrategy` had
//! no variant that could name an Ml sub-op, so nothing could be registered,
//! so `lookup_intrinsic("SAMPLE_TOP_K")` missed.
//!
//! A miss is not an error. `compile_vbc_intrinsic_call` answers one by
//! emitting `Instruction::LoadNil` and returning `Ok`, so the declarations
//! that now live in `core/math/agent.vr` would have compiled cleanly,
//! produced nil at runtime, and reported nothing anywhere — the t0116
//! silent-LoadNil class.
//!
//! These pin the whole route in the one place it is visible at once: the
//! bytecode a `@vbc` call actually compiles to. Asserting the sub-op alone
//! would not be enough — the defect's signature is a `LoadNil` where an
//! envelope should be, so both are checked.

#![cfg(feature = "codegen")]

use verum_ast::FileId;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
use verum_vbc::bytecode::decode_instructions;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::{Instruction, MlSubOpcode, Opcode};
use verum_vbc::module::VbcModule;

fn parse(source: &str) -> verum_ast::Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    VerumParser::new()
        .parse_module(lexer, file_id)
        .unwrap_or_else(|errs| {
            let msgs: Vec<String> = errs.iter().map(|e| format!("{e}")).collect();
            panic!("parse failed:\n{}", msgs.join("\n"))
        })
}

fn compile(source: &str) -> VbcModule {
    let ast = parse(source);
    VbcCodegen::with_config(CodegenConfig::new("m"))
        .compile_module(&ast)
        .unwrap_or_else(|e| panic!("compile failed: {e}"))
}

/// Decode the bytecode of the function whose resolved name ends with
/// `suffix`. Names in the function table are module-qualified, so an exact
/// match on the bare name finds nothing.
fn decoded_fn(module: &VbcModule, suffix: &str) -> Vec<Instruction> {
    let func = module
        .functions
        .iter()
        .find(|f| {
            module
                .strings
                .get(f.name)
                .is_some_and(|n| n == suffix || n.ends_with(&format!(".{suffix}")))
        })
        .unwrap_or_else(|| panic!("function `{suffix}` not found in module"));
    let start = func.bytecode_offset as usize;
    let end = start + func.bytecode_length as usize;
    decode_instructions(&module.bytecode[start..end])
        .unwrap_or_else(|e| panic!("decode of `{suffix}` bytecode failed: {e:?}"))
}

/// The sub-op byte of every MlExtended envelope in `instructions`.
///
/// 0xFD travels as `Instruction::Raw` — the carrier has no typed variant —
/// and the envelope is `[sub_op][varint operand_len][operands]`, so the
/// sub-op is the leading byte of the raw payload.
fn ml_sub_ops(instructions: &[Instruction]) -> Vec<u8> {
    instructions
        .iter()
        .filter_map(|i| match i {
            Instruction::Raw {
                opcode: Opcode::MlExtended,
                data,
            } => data.first().copied(),
            _ => None,
        })
        .collect()
}

#[test]
fn vbc_sample_top_k_compiles_to_the_ml_carrier_not_a_silent_nil() {
    let vbc = compile("fn draw(logits: Int, k: Int) -> Int { @vbc(SAMPLE_TOP_K, logits, k) }");
    let body = decoded_fn(&vbc, "draw");

    assert_eq!(
        ml_sub_ops(&body),
        vec![MlSubOpcode::SampleTopK as u8],
        "expected one MlExtended envelope carrying the top-k sub-op; body: {body:?}"
    );
    assert!(
        !body
            .iter()
            .any(|i| matches!(i, Instruction::LoadNil { .. })),
        "a LoadNil here IS the unregistered-intrinsic path: the call would \
         return nil at runtime with nothing reported. body: {body:?}"
    );
}

#[test]
fn every_ml_serving_intrinsic_reaches_its_own_sub_op() {
    for (call, sub_op) in [
        ("@vbc(SAMPLE_TOP_K, a, b)", MlSubOpcode::SampleTopK),
        (
            "@vbc(SAMPLE_TOP_K_TOP_P, a, b, c)",
            MlSubOpcode::SampleTopKTopP,
        ),
        (
            "@vbc(REPETITION_PENALTY, a, b, c)",
            MlSubOpcode::RepetitionPenalty,
        ),
    ] {
        let vbc = compile(&format!("fn f(a: Int, b: Int, c: Int) -> Int {{ {call} }}"));
        assert_eq!(
            ml_sub_ops(&decoded_fn(&vbc, "f")),
            vec![sub_op as u8],
            "`{call}` must reach its own sub-op"
        );
    }
}
