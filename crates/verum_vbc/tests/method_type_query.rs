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
