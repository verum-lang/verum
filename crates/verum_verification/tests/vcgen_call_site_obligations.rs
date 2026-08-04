//! T0657 — call-site precondition obligations, end to end through
//! the wp engine and the Z3 discharge.
//!
//! Root class: `verum verify` generated ZERO obligations from
//! function bodies. Three cooperating gaps: (1) the live pipeline
//! never asked the wp engine for body obligations; (2) the wp
//! engine's call-site arm only fired with a populated symbol table
//! that no production caller populated; (3) `extract_contract` read
//! only the legacy `@requires` ATTRIBUTE spelling, never the modern
//! `FunctionDecl.requires`/`.ensures` FIELDS. A call violating a
//! callee precondition therefore reported `Proved`.
//!
//! These pins hold the fix at its semantic core: parse REAL Verum
//! syntax, build the module contract table, generate body-obligation
//! VCs, discharge through `HoareZ3Verifier`, and assert both
//! verdict directions plus blame metadata.
//!
//! NOTE: `crates/*/tests/` does not gate in CI until T0709 lands
//! (`cargo test --workspace --lib --bins` skips integration tests);
//! placed here per repository layout policy regardless.

use verum_ast::span::FileId;
use verum_fast_parser::FastParser;
use verum_smt::context::Context as SmtContext;
use verum_verification::vcgen::{Formula, VCGenerator, VCKind};
use verum_verification::{HoareZ3Verifier, LabeledVerificationResult};

/// Parse a module, generate the named function's body-obligation
/// VCs with the module's contract table, and discharge every
/// non-trivial one. Returns the first invalid outcome (with the
/// generator for blame lookups), or None when everything holds.
fn discharge_body_obligations(
    source: &str,
    func_name: &str,
) -> (VCGenerator, Option<LabeledVerificationResult>) {
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect("test source must parse");

    let table = VCGenerator::build_module_contract_table(&module);
    let mut vcgen = VCGenerator::new().with_symbol_table(table);

    let func = module
        .items
        .iter()
        .find_map(|item| match &item.kind {
            verum_ast::ItemKind::Function(fd) if fd.name.as_str() == func_name => Some(fd.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("function '{func_name}' not found in test source"));

    let vcs = vcgen.generate_body_obligation_vcs(&func);

    let ctx = SmtContext::new();
    let verifier = HoareZ3Verifier::new(&ctx).with_timeout(30_000);

    for vc in vcs.iter() {
        let formula = vc.formula.simplify();
        if formula == Formula::True {
            continue;
        }
        let outcome = verifier
            .verify_labeled_formula(&formula)
            .expect("solver must give a verdict on these small formulas");
        if !outcome.valid {
            return (vcgen, Some(outcome));
        }
    }
    (vcgen, None)
}

const DIVIDE: &str = r#"
fn divide(a: Int, b: Int) -> Int requires b != 0 { a / b }
"#;

/// The T0657 repro: a literal 0 argument unconditionally violating
/// `requires b != 0` must FAIL, and the blame must name the
/// call-site precondition.
#[test]
fn literal_zero_argument_fails_with_precondition_blame() {
    let source = format!(
        "{DIVIDE}
fn caller_with_literal_zero() -> Int {{ divide(10, 0) }}
"
    );
    let (vcgen, invalid) = discharge_body_obligations(&source, "caller_with_literal_zero");
    let outcome = invalid.expect("divide(10, 0) against 'requires b != 0' must NOT verify");

    let blamed_kinds: Vec<VCKind> = outcome
        .failed_labels
        .iter()
        .filter_map(|id| vcgen.obligation_meta(*id).map(|m| m.kind))
        .collect();
    assert!(
        blamed_kinds.contains(&VCKind::Precondition),
        "the counterexample model must blame the call-site precondition, got {blamed_kinds:?}"
    );
    let messages: Vec<String> = outcome
        .failed_labels
        .iter()
        .filter_map(|id| vcgen.obligation_meta(*id).map(|m| m.message.to_string()))
        .collect();
    assert!(
        messages.iter().any(|m| m.contains("divide") && m.contains("b != 0")),
        "blame message must name the callee and its requires clause, got {messages:?}"
    );
}

/// A guard establishing the precondition must verify — woven
/// obligations carry their branch context (the pre-fix side-channel
/// pushed the obligation with NO guard in scope).
#[test]
fn guarded_call_verifies() {
    let source = format!(
        "{DIVIDE}
fn guarded(a: Int, b: Int) -> Int {{ if b != 0 {{ divide(a, b) }} else {{ 0 }} }}
"
    );
    let (_, invalid) = discharge_body_obligations(&source, "guarded");
    assert!(
        invalid.is_none(),
        "a call guarded by its exact precondition must verify: {invalid:?}"
    );
}

/// `let`-bound arguments flow through wp substitution: a binding
/// that satisfies the precondition verifies…
#[test]
fn let_bound_satisfying_argument_verifies() {
    let source = format!(
        "{DIVIDE}
fn let_ok(a: Int) -> Int {{ let d = 5; divide(a, d) }}
"
    );
    let (_, invalid) = discharge_body_obligations(&source, "let_ok");
    assert!(
        invalid.is_none(),
        "let d = 5; divide(a, d) must verify — wp folds the tail before the lets: {invalid:?}"
    );
}

/// …and a binding that violates it fails.
#[test]
fn let_bound_violating_argument_fails() {
    let source = format!(
        "{DIVIDE}
fn let_bad(a: Int) -> Int {{ let d = 0; divide(a, d) }}
"
    );
    let (_, invalid) = discharge_body_obligations(&source, "let_bad");
    assert!(
        invalid.is_some(),
        "let d = 0; divide(a, d) must fail the call-site precondition"
    );
}

/// The caller's own `requires` is a hypothesis for its body's
/// obligations (Hoare-triple closure).
#[test]
fn caller_requires_flows_into_obligation() {
    let source = format!(
        "{DIVIDE}
fn pre_flows(a: Int, b: Int) -> Int requires b > 3 {{ divide(a, b) }}
"
    );
    let (_, invalid) = discharge_body_obligations(&source, "pre_flows");
    assert!(
        invalid.is_none(),
        "requires b > 3 implies b != 0 — the obligation must discharge under it: {invalid:?}"
    );
}

/// A caller `requires` too weak to imply the callee's must fail.
#[test]
fn weak_caller_requires_fails() {
    let source = format!(
        "{DIVIDE}
fn pre_weak(a: Int, b: Int) -> Int requires b >= 0 {{ divide(a, b) }}
"
    );
    let (_, invalid) = discharge_body_obligations(&source, "pre_weak");
    assert!(
        invalid.is_some(),
        "b >= 0 admits b == 0 — the call-site precondition must fail"
    );
}

/// Modern clause syntax lands in the contract table:
/// `extract_contract` must read the `requires`/`ensures` FIELDS
/// (the legacy path read only `@requires` attributes — T0657 gap 3).
#[test]
fn modern_clause_syntax_reaches_the_contract_table() {
    let module = FastParser::new()
        .parse_module_str(DIVIDE, FileId::new(0))
        .expect("parse");
    let table = VCGenerator::build_module_contract_table(&module);
    let sig = match table.get_function("divide") {
        verum_common::Maybe::Some(s) => s,
        verum_common::Maybe::None => panic!("divide must be registered in the contract table"),
    };
    assert!(
        sig.precondition != Formula::True,
        "field-declared `requires b != 0` must produce a non-trivial precondition formula"
    );
}

/// Two distinct unmodeled expressions must translate to DISTINCT
/// uninterpreted terms. A shared `unknown()` constant let Z3 prove
/// unrelated expressions equal — a false-`Proved` channel.
#[test]
fn distinct_unknown_expressions_do_not_compare_equal() {
    // `requires` on Text parameters produces formulas over
    // expressions the translator does not model numerically; the
    // obligation `t == u` between two DIFFERENT unknowns must NOT
    // discharge.
    let source = r#"
fn takes(t: Text, u: Text) -> Int requires t == u { 1 }
fn caller(x: Text, y: Text) -> Int { takes(x, y) }
"#;
    // x and y are distinct free variables — `x == y` must not be
    // provable; the point is it must ALSO not be provable when the
    // arguments are expression forms the translator renders as
    // fresh unknowns.
    let (_, invalid) = discharge_body_obligations(source, "caller");
    assert!(
        invalid.is_some(),
        "t == u with unconstrained distinct arguments must not verify"
    );
}
