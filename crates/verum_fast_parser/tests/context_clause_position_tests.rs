#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Regression tests for A88 — one grammar production, three parsers, two of
//! them stricter than the third.
//!
//! `function_def` in `grammar/verum.ebnf` carries `[ context_clause ]`, and
//! two spellings are in use: the canonical post-return
//! `fn f() -> Int using [Database]`, and the pre-return
//! `fn f() using [Database] -> Int` that runs through the L0/vbc/context VCS
//! specs. Three parsers implement the production — free function,
//! `implement` method, protocol method — and only the free function accepted
//! both. Measured 2026-09-05 with six signatures identical but for where
//! they live:
//!
//! ```text
//!   free/pre      parses     impl/pre      E056     protocol/pre   E018
//!   free/post     parses     impl/post     parses   protocol/post  parses
//! ```
//!
//! So a signature written in a free function had to be REWRITTEN to move
//! into an `implement` block, with a diagnostic — "expected '{' or ';' after
//! impl method signature" — that names the brace rather than the clause that
//! actually stopped it. Found in `crates/verum_cli/examples/async_context.vr`
//! (a shipped example) and in `vcs/specs/L2-standard/contexts/README.md`.
//!
//! Fixed by giving all three one door, `parse_optional_context_clause`,
//! called once before `->` and once after; the second call is a no-op when
//! the first consumed a clause, which is also the grammar's "at most one".
//!
//! A80 is the same shape one clause over (`where ensures` accepted anywhere
//! in a free function, only first in an impl method) and is NOT fixed by
//! this change — see the register.
//!
//! WHAT THESE TESTS PIN: that the three positions AGREE, in both orders.
//! Where a spelling is refused everywhere that would be a language decision;
//! what may not happen is one context accepting what another refuses.
//!
//! FIX-SENSITIVITY, MEASURED: reverting the two added pre-return calls fails
//! `impl_method_accepts_the_pre_return_context_clause` and
//! `protocol_method_accepts_the_pre_return_context_clause` and leaves the
//! rest green.

use verum_fast_parser::Parser;

fn parses(src: &str) -> Result<(), String> {
    Parser::new(src)
        .parse_module()
        .map(|_| ())
        .map_err(|e| format!("{:?}", e))
}

const CONTEXT_DECL: &str = "context Database { fn q() -> Int; }\n\n";

/// The control: the free function has accepted both orders all along.
#[test]
fn free_function_accepts_both_context_clause_positions() {
    for (name, src) in [
        ("pre", "fn f() using [Database] -> Int { 1 }"),
        ("post", "fn f() -> Int using [Database] { 1 }"),
    ] {
        let program = format!("{CONTEXT_DECL}{src}\n");
        assert!(
            parses(&program).is_ok(),
            "free function, {name}-return clause: {:?}",
            parses(&program)
        );
    }
}

/// The A88 subject. Before the fix this was
/// `error<E056>: expected '{{' or ';' after impl method signature`.
#[test]
fn impl_method_accepts_the_pre_return_context_clause() {
    let program = format!(
        "{CONTEXT_DECL}type U is {{ }};\n\n\
         implement U {{\n    fn f() using [Database] -> Int {{ 1 }}\n}}\n"
    );
    assert!(
        parses(&program).is_ok(),
        "`using` before `->` in an impl method: {:?}",
        parses(&program)
    );
}

/// The third parser, found while fixing the second — same production, same
/// omission, a different diagnostic (E018, "expected protocol item").
#[test]
fn protocol_method_accepts_the_pre_return_context_clause() {
    let program = format!(
        "{CONTEXT_DECL}type P is protocol {{\n    fn f() using [Database] -> Int;\n}};\n"
    );
    assert!(
        parses(&program).is_ok(),
        "`using` before `->` in a protocol method: {:?}",
        parses(&program)
    );
}

/// The canonical order must keep working in all three — a fix that moved the
/// acceptance rather than widening it would pass the three tests above.
#[test]
fn the_canonical_post_return_position_still_parses_everywhere() {
    let cases = [
        ("free", format!("{CONTEXT_DECL}fn f() -> Int using [Database] {{ 1 }}\n")),
        (
            "impl",
            format!(
                "{CONTEXT_DECL}type U is {{ }};\n\n\
                 implement U {{\n    fn f() -> Int using [Database] {{ 1 }}\n}}\n"
            ),
        ),
        (
            "protocol",
            format!("{CONTEXT_DECL}type P is protocol {{\n    fn f() -> Int using [Database];\n}};\n"),
        ),
    ];
    for (where_, program) in cases {
        assert!(
            parses(&program).is_ok(),
            "canonical post-return clause in a {where_} signature: {:?}",
            parses(&program)
        );
    }
}

/// NEGATIVE CONTROL. The grammar allows AT MOST ONE context clause, and the
/// pre-return leg must not have turned the door into a repeatable one.
#[test]
fn two_context_clauses_on_one_signature_are_still_refused() {
    let program =
        format!("{CONTEXT_DECL}fn f() using [Database] -> Int using [Database] {{ 1 }}\n");
    assert!(
        parses(&program).is_err(),
        "a signature carrying the clause in BOTH positions must be refused"
    );
}
