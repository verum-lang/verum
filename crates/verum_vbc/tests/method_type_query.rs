//! Receiver recovery must share each subtree's facts, including failed lookups.
#![cfg(feature = "codegen")]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

#[test]
fn unresolved_method_chains_finish_within_the_query_budget() {
    const CHILD: &str = "VERUM_METHOD_QUERY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let codegen = VbcCodegen::with_config(CodegenConfig::new("query"));
        for depth in [0, 1, 8, 32, 128] {
            let source = format!("unresolved{}", ".missing()".repeat(depth));
            let expression = Parser::new(&source).parse_expr().expect("parse chain");
            assert_eq!(codegen.extract_expr_type_name(&expression), None);
        }
        return;
    }
    // A regression must fail finitely instead of leaving an exponentially
    // expanding query alive in the test runner. This is a wall-time guard,
    // not an instruction counter or a claim about asymptotic complexity.
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "unresolved_method_chains_finish_within_the_query_budget",
        ])
        .env(CHILD, "1")
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "receiver query child failed: {status}");
            break;
        }
        if start.elapsed() >= Duration::from_secs(10) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("unresolved receiver queries exceeded 10 seconds");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn long_generic_chains_keep_the_declared_result() {
    let source = Parser::new(
        r#"
module query;
type Chain<T> is { value: T };
implement<T> Chain<T> {
    fn new(value: T) -> Self { Chain { value } }
    fn next(self) -> Self { self }
    fn finish(self) -> T { self.value }
}

"#,
    )
    .parse_module()
    .expect("parse declarations");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("query"));
    codegen
        .compile_module(&source)
        .expect("compile declarations");
    for (ty, value) in [("Int", "42"), ("Text", "\"text\"")] {
        for depth in [0, 1, 8, 32] {
            let source = format!(
                "Chain<{ty}>.new({value}){}.finish()",
                ".next()".repeat(depth)
            );
            let expression = Parser::new(&source)
                .parse_expr()
                .expect("parse typed chain");
            assert_eq!(
                codegen.extract_expr_type_name(&expression).as_deref(),
                Some(ty),
                "{source}"
            );
        }
    }
}

fn declarations(source: &str) -> VbcCodegen {
    let ast = Parser::new(source)
        .parse_module()
        .expect("parse declarations");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("authority"));
    codegen.compile_module(&ast).expect("compile declarations");
    codegen
}

#[test]
fn unrelated_methods_cannot_supply_an_unresolved_receivers_result() {
    let other = "type Other is {}; implement Other { fn reveal(&self) -> Int { 42 } }";
    for extra in [
        "",
        other,
        &format!("{other} type Same is {{}}; implement Same {{ fn reveal(&self) -> Int {{ 7 }} }}"),
        &format!(
            "{other} type Different is {{}}; implement Different {{ fn reveal(&self) -> Bool {{ true }} }}"
        ),
    ] {
        let codegen = declarations(&format!("module authority; {extra}"));
        for query in ["unresolved().reveal()", "(unresolved().reveal()).reveal()"] {
            let expression = Parser::new(query).parse_expr().expect("parse query");
            assert_eq!(
                codegen.extract_expr_type_name(&expression),
                None,
                "{query}: {extra}"
            );
        }
    }
}

#[test]
fn method_result_requires_a_declaration_even_for_familiar_names() {
    let codegen = declarations(
        r#"
module authority;
type Container<T> is { value: T };
fn make() -> Container<Int> { Container { value: 42 } }
"#,
    );
    for query in [
        "Container.missing()",
        "Container.parse(42)",
        "Container.from_digit(42)",
        "Container<Int>.new(42)",
        "make().unwrap()",
        "make().unwrap_err()",
        "make().add(1)",
        "make().offset(1)",
    ] {
        let expression = Parser::new(query).parse_expr().expect("parse query");
        assert_eq!(codegen.extract_expr_type_name(&expression), None, "{query}");
    }
}

#[test]
fn familiar_method_names_keep_their_actual_declared_results() {
    let codegen = declarations(
        r#"
module authority;
type Container<T> is { value: T };
implement<T> Container<T> {
    fn parse(value: T) -> Bool { true }
    fn unwrap(&self) -> Bool { false }
    fn add(&self, value: Int) -> Bool { true }
}
fn make() -> Container<Int> { Container { value: 42 } }
"#,
    );
    for query in [
        "Container<Int>.parse(42)",
        "make().unwrap()",
        "make().add(1)",
    ] {
        let expression = Parser::new(query).parse_expr().expect("parse query");
        assert_eq!(
            codegen.extract_expr_type_name(&expression).as_deref(),
            Some("Bool"),
            "{query}"
        );
    }
}

#[test]
fn alias_receivers_preserve_declared_type_and_const_arguments() {
    let codegen = declarations(
        r#"
module authority;
type Alloc<const SIZE: Int> is { top: Int };
implement<const SIZE: Int> Alloc<SIZE> {
    fn new() -> Self { Self { top: 0 } }
    fn capacity(&self) -> Int { SIZE }
}
type Tiny is Alloc<1024>;
type Small is Tiny;
type Pair<A, B> is { left: A, right: B };
implement<A, B> Pair<A, B> {
    fn new(left: A, right: B) -> Self { Self { left, right } }
    fn second(self) -> B { self.right }
}
type Swapped<X, Y> is Pair<Y, X>;
"#,
    );
    for (query, expected) in [
        ("Tiny.new()", "Alloc<1024>"),
        ("Small.new()", "Alloc<1024>"),
        ("Tiny.new().capacity()", "Int"),
        ("Swapped<Bool, Int>.new(1, true)", "Pair<Int, Bool>"),
        ("Swapped<Bool, Int>.new(1, true).second()", "Bool"),
    ] {
        let expression = Parser::new(query).parse_expr().expect("parse alias query");
        assert_eq!(
            codegen.extract_expr_type_name(&expression).as_deref(),
            Some(expected),
            "{query}"
        );
    }
    for query in ["Tiny.missing()", "Swapped<Bool, Int>.parse(1)"] {
        let expression = Parser::new(query)
            .parse_expr()
            .expect("parse absent alias method");
        assert_eq!(codegen.extract_expr_type_name(&expression), None, "{query}");
    }
}
