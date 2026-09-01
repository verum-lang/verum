//! An intrinsic called with the wrong number of arguments must be a
//! compile error, not a dropped function (T0693 / T1056).
//!
//! `compile_imported_intrinsic_call` used to compile as many arguments
//! as it was given and emit one register per argument. The AOT lowering
//! reads the operands the OPCODE declares, so a short call produced a
//! short operand stream and died decoding it:
//!
//!     Skipping function 'Channel.new':
//!         Internal("read_reg_varlen: operand stream exhausted")
//!
//! The function was then dropped from the binary with a WARN nothing
//! counts, while the interpreter — which reads what is there — ran it
//! fine. `core/async/channel.vr` called `alloc(buf_size)` on the
//! 2-parameter `alloc` for exactly this reason, and `Mutex`-free
//! programs using `Channel` printed a value under Tier 0 and SIGSEGVed
//! under Tier 1.
//!
//! The sibling gate `declared_arities_match_the_registry` covers the
//! `@intrinsic("key")` DECLARATION side. This covers the CALL side,
//! which nothing did: a bare intrinsic name resolves straight through
//! `lookup_intrinsic` without consulting a declaration at all.
//!
//! WHY THIS IS A UNIT TEST AND NOT A SOURCE SCAN: a static scan of
//! `core/` cannot tell an intrinsic call from a same-named local
//! function, and it reads text a compiler never sees. Measured — a scan
//! reported twelve mismatches, and every one was a false positive:
//! `log(b, x)` inside a trailing comment, `type_name(v)` resolving to a
//! locally declared `public fn type_name`, and six legal `panic()`
//! calls. Name resolution is the fact, and only the compiler holds it.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

fn compile(source: &str) -> Result<(), String> {
    let mut parser = Parser::new(source);
    let module_ast = parser
        .parse_module()
        .map_err(|e| format!("parse failed: {:?}", e))?;
    let config = CodegenConfig::new("arity_probe.vr");
    let mut codegen = VbcCodegen::with_config(config);
    codegen
        .compile_module_with_mounts(&module_ast, "arity_probe.vr", ".")
        .map_err(|e| format!("{:?}", e))?;
    Ok(())
}

/// The exact shape that shipped a binary without `Channel.new`.
#[test]
fn a_short_intrinsic_call_is_refused() {
    let source = r#"
module probe.arity;

public fn make(n: Int) -> Int {
    let p = alloc(n);
    0
}
"#;
    let err = compile(source).expect_err(
        "`alloc(n)` passes one argument to a two-parameter intrinsic and must \
         not compile — the emitted operand stream does not decode at AOT \
         lowering and the enclosing function is dropped from the binary",
    );
    assert!(
        err.contains("alloc") && err.contains('2') && err.contains('1'),
        "the diagnostic must name the intrinsic and BOTH counts so the author \
         can act on it; got: {}",
        err
    );
}

/// Positive control: the same call with the declared arity compiles.
/// Without this the test above would pass just as well if `alloc` had
/// stopped resolving as an intrinsic at all.
#[test]
fn a_correct_intrinsic_call_still_compiles() {
    let source = r#"
module probe.arity_ok;

public fn make(n: Int) -> Int {
    let p = alloc(n, 8);
    0
}
"#;
    compile(source).expect("`alloc(n, 8)` matches the registry arity and must compile");
}

/// The over-long direction too — an extra argument is the same contract
/// violation and produces a stream the lowering reads past.
#[test]
fn an_over_long_intrinsic_call_is_refused() {
    let source = r#"
module probe.arity_long;

public fn make(n: Int) -> Int {
    let p = alloc(n, 8, 16);
    0
}
"#;
    let err = compile(source).expect_err("three arguments to a two-parameter intrinsic");
    assert!(
        err.contains("alloc"),
        "the diagnostic must name the intrinsic; got: {}",
        err
    );
}
