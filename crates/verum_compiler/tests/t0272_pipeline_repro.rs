//! T0272: reliable in-process repro of the Int128 pipeline truncation via the
//! full compiler pipeline (compile_to_vbc = type-check+codegen; compile_script_to_vbc
//! = the exact path `verum run` uses). Avoids the verum_interactive build.
//!
//! The `_typecheck_path` variants run the minimal-type-check compile
//! (`compile_to_vbc`); the `_verumrun_path` / `script_*` variants run the
//! stdlib-linked interpreter pipeline (`compile_script_to_vbc`) — the path that
//! `verum run` uses and that dropped the 128-bit width before this fix.
use std::sync::Arc;
use verum_vbc::interpreter::Interpreter;

fn run_main_i128(vbc: verum_vbc::VbcModule) -> (bool, u128) {
    let m = Arc::new(vbc);
    // Mirror `ScriptEngine::run` — the EXACT entry resolution `verum run` uses:
    // exact "main", else the unique bare-suffix "<mod>.main". NOT `FunctionId(0)`
    // (in a stdlib-linked module that is a stdlib function, not the user main)
    // and NOT a naive `.ends_with(".main")` scan (which `.find()`s whatever
    // main-suffixed function is registered first — the source of the earlier
    // all-`0x0` script-path results that masqueraded as a codegen truncation).
    let func_id = m
        .find_function_by_name("main")
        .or_else(|| m.find_function_by_unique_bare_suffix("main"))
        .expect("main entry not found");
    let mut interp = Interpreter::new(m);
    let result = interp
        .execute_function_with_args(func_id, &[])
        .expect("execution failed");
    (result.is_boxed_i128(), result.as_i128_raw())
}

/// Compile through the exact `verum run` path (stdlib-linked) and run `main`.
fn script_i128(src: &str) -> (bool, u128) {
    let vbc = verum_compiler::api::compile_script_to_vbc(src).expect("compile_script_to_vbc failed");
    run_main_i128(vbc)
}

/// Compile through the minimal type-check path and run `main`.
fn typecheck_i128(src: &str) -> (bool, u128) {
    let vbc = verum_compiler::api::compile_to_vbc(src).expect("compile_to_vbc failed");
    run_main_i128(vbc)
}

const SRC: &str = "fn main() -> Int128 { let a: Int128 = 4000000000; a * a }";
const EXPECTED: u128 = 16_000_000_000_000_000_000; // 4e9^2 > i64::MAX

#[test]
fn t0272_compile_to_vbc_typecheck_path() {
    let (boxed, raw) = typecheck_i128(SRC);
    assert!(boxed, "compile_to_vbc: expected boxed Int128, was not boxed (raw={:#x})", raw);
    assert_eq!(raw, EXPECTED, "compile_to_vbc TRUNCATED: {:#x}", raw);
}

#[test]
fn t0272_compile_script_to_vbc_verumrun_path() {
    let (boxed, raw) = script_i128(SRC);
    assert!(boxed, "compile_script_to_vbc (verum-run path): not boxed (raw={:#x})", raw);
    assert_eq!(raw, EXPECTED, "compile_script_to_vbc TRUNCATED: {:#x}", raw);
}

// ---- Full-surface coverage through the stdlib-linked (`verum run`) path. ----
// Each mirrors a case in scratchpad/t0602/verify_t0272.vr, but returns the
// value so the harness reads the raw 128-bit result instead of a formatted
// string. All must survive the pipeline at full width.

#[test]
fn t0272_script_wide_literal_round_trips() {
    // i128::MAX — larger than u64, must not collapse to -1 / a 64-bit value.
    let (boxed, raw) = script_i128(
        "fn main() -> Int128 { let big: Int128 = 170141183460469231731687303715884105727; big }",
    );
    assert!(boxed, "wide literal not boxed (raw={:#x})", raw);
    assert_eq!(raw, 170141183460469231731687303715884105727u128, "wide literal TRUNCATED: {:#x}", raw);
}

#[test]
fn t0272_script_add_overflowing_i64() {
    // i64::MAX + i64::MAX = 18446744073709551614 (overflows i64, fits u64).
    let (boxed, raw) = script_i128(
        "fn main() -> Int128 { let a: Int128 = 9223372036854775807; a + a }",
    );
    assert!(boxed, "add result not boxed (raw={:#x})", raw);
    assert_eq!(raw, 18_446_744_073_709_551_614u128, "add TRUNCATED: {:#x}", raw);
}

#[test]
fn t0272_script_mul_beyond_u64() {
    // 10^12 * 10^12 = 10^24 — far beyond u64.
    let (boxed, raw) = script_i128(
        "fn main() -> UInt128 { let c: UInt128 = 1000000000000; c * c }",
    );
    assert!(boxed, "mul result not boxed (raw={:#x})", raw);
    assert_eq!(raw, 1_000_000_000_000_000_000_000_000u128, "mul TRUNCATED: {:#x}", raw);
}

#[test]
fn t0272_script_shift_beyond_u64() {
    // (2^64) >> 64 = 1 — proves the 2^64 literal survived as a 128-bit value.
    let (boxed, raw) = script_i128(
        "fn main() -> UInt128 { let f: UInt128 = 18446744073709551616; f >> 64 }",
    );
    assert!(boxed, "shift result not boxed (raw={:#x})", raw);
    assert_eq!(raw, 1u128, "shift wrong: {:#x}", raw);
}

#[test]
fn t0272_script_small_operands_still_work() {
    // Small values annotated Int128 must still compute correctly (100 * 3 = 300).
    let (_boxed, raw) = script_i128(
        "fn main() -> Int128 { let sm: Int128 = 100; sm * 3 }",
    );
    assert_eq!(raw, 300u128, "small Int128 arithmetic wrong: {:#x}", raw);
}
