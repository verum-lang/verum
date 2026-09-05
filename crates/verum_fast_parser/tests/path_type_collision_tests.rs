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
//! Regression tests for A70 — `Path<T>` is claimed by the cubical-path
//! production, and the diagnostic did not say so.
//!
//! `grammar/verum.ebnf` defines `path_type_expr` as `Path`, type arguments,
//! and TWO parenthesised endpoints, so a parser reading `Path<Int>` demands
//! `(a, b)`. Measured 2026-09-04/05 on identical code differing only in the
//! type name: `Path<Int>` gives E018, while `Ctx`, `Query`, `Json`, `State`
//! and `Header` all reach type-checking. `type Path<T> is (T);` PARSES and
//! every use of it is refused — the definition looks accepted, which is worse
//! than an outright rejection.
//!
//! The name is genuinely in use: `core/math/cubical.vr` writes `Path<A>(a, b)`
//! in seventeen non-comment positions. So the production is not vestigial and
//! cannot be gated away, and the row's stated acceptance is a DIAGNOSTIC that
//! names the collision — not a language change.
//!
//! WHAT THESE TESTS PIN: that the message says `Path` is reserved and shows
//! the cubical spelling, that the bare name is still free, and that a real
//! cubical path still parses.

use verum_fast_parser::Parser;

fn parse_err(src: &str) -> Option<String> {
    Parser::new(src).parse_module().err().map(|e| format!("{e:?}"))
}

/// The subject: a generic `Path<T>` in parameter position.
#[test]
fn path_with_type_args_and_no_endpoints_names_the_collision() {
    let err = parse_err("fn f(p: Path<Int>) -> Int { 1 }\n")
        .expect("`Path<Int>` without endpoints must still be refused");
    assert!(
        err.contains("Path") && err.contains("cubical"),
        "the diagnostic must name what collided; got: {err}"
    );
}

/// Same in return position and in a `let` annotation — all three were E018
/// with no mention of the collision.
#[test]
fn every_position_gets_the_same_explanation() {
    for src in [
        "fn f() -> Path<Int> { todo() }\n",
        "fn f() -> Int { let p: Path<Int> = todo(); 1 }\n",
    ] {
        let err = parse_err(src).unwrap_or_default();
        assert!(
            err.contains("cubical"),
            "position must not change the explanation; got: {err}"
        );
    }
}

/// NEGATIVE CONTROL 1. A real cubical path must still parse — the change is a
/// diagnostic on the failing branch, not a restriction.
#[test]
fn a_real_cubical_path_still_parses() {
    let src = "fn f(p: Path<Int>(1, 1)) -> Int { 1 }\n";
    assert!(
        parse_err(src).is_none(),
        "`Path<A>(a, b)` is the production this name exists for: {:?}",
        parse_err(src)
    );
}

/// NEGATIVE CONTROL 2. The BARE name is free — `core/io/path.vr` declares
/// `public type Path` with no parameters, which is why the stdlib never trips
/// over this.
#[test]
fn the_bare_name_path_is_still_an_ordinary_type() {
    let src = "fn f(p: Path) -> Int { 1 }\n";
    assert!(
        parse_err(src).is_none(),
        "the bare `Path` must stay usable: {:?}",
        parse_err(src)
    );
}
