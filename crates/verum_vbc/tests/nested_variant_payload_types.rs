#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

fn run(source: &str, result_type: Option<&str>) -> i64 {
    let source = format!("type Poll<T> is Pending | Ready(T);\n{source}\nfn after_probe() {{}}");
    let ast = Parser::new(&source).parse_module().expect("parse");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("nested_payload_pins"));
    let module = codegen.compile_module(&ast).expect("compile");
    if let Some(expected) = result_type {
        assert_eq!(
            codegen
                .variable_type_names()
                .get("result")
                .map(String::as_str),
            Some(expected),
            "match result must carry the nested payload's type"
        );
    }
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    let value = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .expect("execute");
    assert!(value.is_int(), "expected Int, got {value:?}");
    value.as_i64()
}

#[test]
fn nested_record_binding_uses_its_own_field_layout() {
    assert_eq!(
        run(
            r#"
type Decoy is { answer: Int, padding: Int, extra: Int };
type Answer is { padding: Int, answer: Int };
fn probe() -> Int {
    let input: Poll<Result<Answer, Bool>> = Poll.Ready(Result.Ok(Answer { padding: 99, answer: 7 }));
    match input { Poll.Ready(Result.Ok(value)) => value.answer, _ => -1 }
}
"#,
            None
        ),
        7
    );
}

#[test]
fn nested_record_match_result_preserves_nominal_identity() {
    assert_eq!(
        run(
            r#"
type Decoy is { answer: Int, padding: Int, extra: Int };
type Answer is { padding: Int, answer: Int };
fn probe() -> Int {
    let input: Poll<Result<Answer, Bool>> = Poll.Ready(Result.Ok(Answer { padding: 99, answer: 7 }));
    let result = match input {
        Poll.Ready(Result.Ok(value)) => value,
        _ => Answer { padding: 88, answer: -1 },
    };
    result.answer
}
"#,
            Some("Answer")
        ),
        7
    );
}

#[test]
fn nested_int_match_result_carries_int_and_formats_as_int() {
    assert_eq!(
        run(
            r#"
fn probe() -> Int {
    let input: Poll<Result<Int, Bool>> = Poll.Ready(Result.Ok(1));
    let result = match input { Poll.Ready(Result.Ok(value)) => value, _ => -1 };
    let displayed = f"{result}";
    if displayed == "1" { 7 } else { -1 }
}
"#,
            Some("Int")
        ),
        7
    );
}

#[test]
fn nested_err_payload_uses_the_error_argument() {
    assert_eq!(
        run(
            r#"
type Decoy is { answer: Int, padding: Int, extra: Int };
type Answer is { padding: Int, answer: Int };
fn probe() -> Int {
    let input: Poll<Result<Bool, Answer>> = Poll.Ready(Result.Err(Answer { padding: 99, answer: 7 }));
    match input { Poll.Ready(Result.Err(value)) => value.answer, _ => -1 }
}
"#,
            None
        ),
        7
    );
}

#[test]
fn nested_context_is_restored_for_the_next_payload() {
    assert_eq!(
        run(
            r#"
type Decoy is { answer: Int, padding: Int, extra: Int };
type Answer is { padding: Int, answer: Int };
type Other is { answer: Int, padding: Int };
type Pair<A, B> is Both(A, B);
fn probe() -> Int {
    let input: Pair<Result<Answer, Bool>, Other> = Pair.Both(
        Result.Ok(Answer { padding: 99, answer: 3 }), Other { answer: 4, padding: 88 }
    );
    match input { Pair.Both(Result.Ok(value), other) => value.answer + other.answer, _ => -1 }
}
"#,
            None
        ),
        7
    );
}

#[test]
fn a_different_nested_variant_still_takes_the_fallback() {
    assert_eq!(
        run(
            r#"
type Answer is { padding: Int, answer: Int };
fn probe() -> Int {
    let input: Poll<Result<Answer, Bool>> = Poll.Ready(Result.Err(false));
    match input { Poll.Ready(Result.Ok(value)) => value.answer, _ => 7 }
}
"#,
            None
        ),
        7
    );
}
