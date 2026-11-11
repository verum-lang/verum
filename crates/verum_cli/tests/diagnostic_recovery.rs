//! An invalid call must not disable ownership diagnostics in subsequent source.
use std::{fs, process::Command};

fn check(body: &str, accepted: bool, expected: &[&str]) {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module prelude {}").unwrap();
    let entry = root.path().join("main.vr");
    fs::write(
        &entry,
        format!(
            "@no_implicit_prelude\nmodule app.main;\n\
         type affine Parcel is {{value: Int}};\n\
         fn need(x: Int) {{}}\n{body}\nfn main() {{}}"
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_verum"))
        .arg("check")
        .arg(&entry)
        .current_dir(root.path())
        .env("VERUM_STDLIB_PATH", core)
        .env("VERUM_ALLOW_STDLIB_SOURCE_FALLBACK", "1")
        .env("VERUM_NO_OBJECT_CACHE", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let messages = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.success(), accepted, "{messages}");
    for message in expected {
        assert!(messages.contains(message), "missing {message}: {messages}");
    }
}

#[test]
fn match_coverage_uses_real_guards_and_exact_numeric_domains() {
    for body in [
        "fn bad(value: Int) -> Int {match value {0 => 5, _ if false => 99}}",
        "fn bad(value: Int) {let result = match value {0 => 5, 1 => 99};}",
        "fn bad(value: Bool) -> Int {match value {true if false => 1, false => 0}}",
        "fn bad(value: &Bool) -> Int {match value {false => 0}}",
    ] {
        check(body, false, &["E0601"]);
    }
    check(
        "fn valid(value: Int) -> Int {match value {..0 => 1, 0.. => 2}}",
        true,
        &[],
    );
    check(
        "fn valid(value: Bool) -> Int {match value {true if false => 1, _ => 2}}",
        true,
        &[],
    );
}

#[test]
fn refinement_coverage_requires_every_admissible_value() {
    check(
        "fn bad(value: Int{it >= 0 && it <= 100}) -> Int {
        match value {_ if false => 99, 0 => 0, 50 => 1, 100 => 2}}",
        false,
        &["E0601", "coverage is not established"],
    );
    check(
        "fn valid(value: Int{it >= 0 && it <= 100}) -> Int {
        match value {0..=100 => 1}}",
        true,
        &[],
    );
    check(
        "fn unknown(value: Int{it % 2 == 0}) -> Int {
        match value {0 => 1}}",
        false,
        &["E0601", "coverage is not established"],
    );
}

#[test]
fn invalid_calls_preserve_following_ownership_diagnostics() {
    for body in [
        "fn bad(original: Parcel) {need(true); (original, original);}",
        "fn bad() {need(true);} fn later(original: Parcel) {(original, original);}",
        "fn bad(original: Parcel) {(|x: Int| x)(true); (original, original);}",
    ] {
        check(body, false, &["Bool", "Int", "E310", "used after move"]);
    }
    check(
        "fn valid(original: Parcel) {need(1); let moved = original; moved.value;}",
        true,
        &[],
    );
}

#[test]
fn invalid_blocks_do_not_authorize_later_source() {
    check(
        "fn bad() {unsafe {missing}; true as (Int, Int);}",
        false,
        &["E100", "E401", "types are not compatible for casting"],
    );
    check(
        "fn bad() {async {missing}; (1).await;}",
        false,
        &["E100", "async context"],
    );
    check(
        "fn bad() {{let hidden = 1; missing}; hidden;}",
        false,
        &["unbound variable: missing", "unbound variable: hidden"],
    );
    check("fn valid() {async {async {1}; (1).await};}", true, &[]);
}

#[test]
fn callable_bodies_preserve_ownership_and_async_boundaries() {
    check(
        "fn accept(f: fn(Parcel) -> (Parcel, Parcel)) {}
         fn bad() {accept(|item| (item, item));}",
        false,
        &["E310", "used after move"],
    );
    check(
        "fn bad(original: Parcel) {fn broken() -> Int {missing} (original, original);}",
        false,
        &["E100", "E310"],
    );
    check(
        "fn bad() {async |value: Int| missing; (1).await;}",
        false,
        &["E100", "async context"],
    );
    check(
        "fn accept(f: fn(Parcel) -> Parcel) {}
         fn valid() {accept(|item| item);}",
        true,
        &[],
    );
}

#[test]
fn annotated_closure_bindings_use_subtyping_without_discarding_results() {
    check(
        "fn valid() -> Int {let base = 10; let f: fn(Int) -> Int = |x| x + base; f(5)}",
        true,
        &[],
    );
    check("fn bad() {let f: fn() = || -> Int {1};}", false, &["E400"]);
    check(
        "fn bad() {let f: fn(Int) -> Bool = |x| -> Int {x};}",
        false,
        &["E400"],
    );
}

#[test]
fn provided_contexts_obey_lexical_boundaries_during_recovery() {
    const DECLARATIONS: &str = "context Gate {fn ping();}
        type Channel is {value: Int};
        implement Channel {fn ping(&self) {}}
        fn needs() using [Gate] {Gate.ping();}";
    for provision in [
        "{provide Gate = provider; needs();}",
        "provide Gate = provider in {needs();}",
        "{provide Gate = provider; missing}",
        "provide Gate = provider in {missing}",
    ] {
        check(
            &format!("{DECLARATIONS} fn bad(provider: Channel) {{{provision} needs();}}"),
            false,
            &["E613"],
        );
    }
    check(
        &format!("{DECLARATIONS} fn bad(provider: Channel) {{provide Missing = provider in {{}}}}"),
        false,
        &["E605"],
    );
    check(
        &format!(
            "{DECLARATIONS} fn valid(provider: Channel) {{provide Gate = provider;
            {{provide Gate = provider; needs();}} needs();}}"
        ),
        true,
        &[],
    );
}

#[test]
fn nested_functions_must_own_their_context_requirements() {
    const DECLARATIONS: &str = "context Gate {} fn needs() using [Gate] {}";
    check(
        &format!(
            "{DECLARATIONS} fn bad() {{provide Gate = ();
            fn hidden() {{needs();}} hidden();}}"
        ),
        false,
        &["E613"],
    );
    check(
        &format!(
            "{DECLARATIONS} fn valid() {{provide Gate = ();
            fn nested() using [Gate] {{needs();}} nested(); needs();}}"
        ),
        true,
        &[],
    );
}

#[test]
fn recovery_handlers_keep_their_bindings_and_guard_contracts_local() {
    check(
        "fn bad() -> Int {let value: Int = try {1} recover |hidden| {2}; hidden}",
        false,
        &["E100", "unbound variable: hidden"],
    );
    check(
        "fn bad() -> Int {try {1} recover {hidden if true => 2, _ => hidden}}",
        false,
        &["E100", "unbound variable: hidden"],
    );
    check(
        "fn bad() {let value = try {1} recover {_ if 7 => 2, _ => 3};}",
        false,
        &["E400", "Bool", "Int"],
    );
    check(
        "fn valid() -> Int {let hidden = 7;
            let value: Int = try {1} recover |hidden| {2}; hidden + value}",
        true,
        &[],
    );
}
