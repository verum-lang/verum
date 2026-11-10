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

#[test]
fn nested_variant_tests_use_the_instantiated_field_owner() {
    assert_eq!(
        run(
            r#"
type Answer is Busy | ReadyNow;
type Decoy is ReadyNow | Ignored;
type Pair<A, B> is First(A) | Second(B);
fn classify(value: Pair<Decoy, Answer>) -> Int {
    match value {
        Pair.Second(ReadyNow) => 40,
        Pair.Second(Busy) => 3,
        _ => 1,
    }
}
fn probe() -> Int {
    classify(Pair.Second(Answer.ReadyNow)) +
    classify(Pair.Second(Answer.Busy)) +
    classify(Pair.First(Decoy.ReadyNow))
}
"#,
            None,
        ),
        44
    );
}

#[test]
fn unresolved_variant_patterns_have_no_runtime_tag() {
    let ast = Parser::new(
        "type State is Open | Closed; fn test(s: State) -> Bool { match s { Unresolved => true, _ => false } }",
    )
    .parse_module()
    .unwrap();
    let error = VbcCodegen::with_config(CodegenConfig::new("pattern_test"))
        .compile_module(&ast)
        .expect_err("unresolved variant name must not become a discriminant");
    assert!(error.to_string().contains("Unresolved"), "{error}");
}

#[test]
fn tuple_patterns_use_each_elements_declared_variant_order() {
    assert_eq!(
        run(
            r#"
type Left is Other | Hit;
type Right is Hit | Other;
fn classify(left: Left, right: Right) -> Int {
    match (left, right) {
        (Hit, Hit) => 7,
        (Hit, Other) => 5,
        (Other, Hit) => 3,
        (Other, Other) => 1,
    }
}
fn nested(pair: (Right, Left), left: Left) -> Int {
    match (left, pair) { (Hit, (Hit, Other)) => 2, _ => 0 }
}
fn probe() -> Int {
    classify(Left.Hit, Right.Hit) * 1000
    + classify(Left.Hit, Right.Other) * 100
    + classify(Left.Other, Right.Hit) * 10
    + classify(Left.Other, Right.Other)
    + nested((Right.Hit, Left.Other), Left.Hit)
}
"#,
            None,
        ),
        7533
    );
}

#[test]
fn writing_a_borrowed_scalar_or_record_updates_the_original_variant() {
    let source = r#"
        module projection;
        type Cell is { value: Int };
        type Choice<T> is Empty | Value(T);
        fn set_scalar(sum: &mut Choice<Int>) {
            match sum { Choice.Value(value) => { *value = 73; }, Choice.Empty => {}, }
        }
        fn set_record(sum: &mut Choice<Cell>) {
            match sum { Choice.Value(cell) => { cell.value = 41; }, Choice.Empty => {}, }
        }
        public fn observe() -> Int {
            let mut a = Choice.Value(5);
            let mut b = Choice.Value(Cell { value: 7 });
            set_scalar(&mut a);
            set_record(&mut b);
            let left = match a { Choice.Value(value) => value, Choice.Empty => 0 };
            let right = match b { Choice.Value(cell) => cell.value, Choice.Empty => 0 };
            left + right
        }
    "#;
    let syntax = Parser::new(source).parse_module().unwrap();
    let module = VbcCodegen::new().compile_module(&syntax).unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    let module = verum_vbc::deserialize::deserialize_module(&wire).unwrap();
    verum_vbc::validate::validate_module(&module).unwrap();
    let observe = module.find_function_by_name("projection.observe").unwrap();
    let value = Interpreter::new(Arc::new(module))
        .execute_function(observe).unwrap();
    assert_eq!(value.try_as_i64(), Some(114));
}
