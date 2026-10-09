//! T1687: expression type hints cannot choose a function's residual return target.
//! Parse, lower, serialize and execute local declarations without a stdlib archive.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_common::Text;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

// These local methods reproduce the standard library's two cross-carrier
// residual conversions. Same-carrier propagation must not call either one.
const DECLARATIONS: &str = r#"
type Failure is { code: Int };
type ConvertedFailure is { code: Int };
type Built is { value: Maybe<Int> };
type Nested is { built: Built };
type ResultField is { value: Result<Int, Failure> };
implement<T, E> Maybe<T> {
    fn from_residual(_residual: Result<Never, E>) -> Maybe<T> { Maybe.None }
}
implement<T> Result<T, Failure> {
    fn from_residual(_residual: Maybe<Never>) -> Result<T, Failure> {
        Result.Err(Failure { code: 83 })
    }
}
implement ConvertedFailure {
    fn from(error: Failure) -> ConvertedFailure {
        ConvertedFailure { code: error.code + 100 }
    }
}
fn missing() -> Result<Maybe<Int>, Failure> { Result.Err(Failure { code: 41 }) }
fn present() -> Result<Maybe<Int>, Failure> { Result.Ok(Maybe.Some(7)) }
fn nullable() -> Result<Maybe<Int>, Failure> { Result.Ok(Maybe.None) }
fn missing_result() -> Result<Result<Int, Failure>, Failure> {
    Result.Err(Failure { code: 41 })
}
fn absent() -> Maybe<Int> { Maybe.None }
fn accept(value: Maybe<Int>) -> Int {
    match value { Maybe.Some(number) => number, Maybe.None => 9 }
}
"#;

fn value(body: &str, expected: i64) {
    let source: Text = format!("{DECLARATIONS}\n{body}").into();
    let ast = Parser::new(source.as_str())
        .parse_module()
        .expect("source grammar");
    let module = VbcCodegen::with_config(CodegenConfig::new("residual_boundaries"))
        .compile_module(&ast)
        .expect("source VBC");
    let bytes = verum_vbc::serialize::serialize_module(&module).expect("serialize VBC");
    let module = verum_vbc::deserialize::deserialize_module(&bytes).expect("decode VBC");
    let entry = module
        .functions
        .iter()
        .find(|function| {
            module
                .get_string(function.name)
                .is_some_and(|name| name == "probe" || name.ends_with(".probe"))
        })
        .expect("qualified probe")
        .id;
    let actual = Interpreter::new(Arc::new(module))
        .execute_function(entry)
        .expect("source must execute");
    assert!(actual.is_int(), "expected Int, got {actual:?}");
    assert_eq!(actual.as_i64(), expected);
}

#[test]
fn propagation_before_record_construction_preserves_the_error() {
    value(
        r#"
fn build() -> Result<Built, Failure> {
    let value = missing()?;
    Result.Ok(Built { value })
}
fn probe() -> Int { match build() { Result.Ok(_) => 0, Result.Err(error) => error.code } }
"#,
        41,
    );
}

#[test]
fn a_maybe_record_field_cannot_convert_the_enclosing_result_error_to_none() {
    value(
        r#"
fn build() -> Result<Built, Failure> { Result.Ok(Built { value: missing()? }) }
fn probe() -> Int { match build() { Result.Ok(_) => 0, Result.Err(error) => error.code } }
"#,
        41,
    );
}

#[test]
fn a_successful_present_field_keeps_its_payload() {
    value(
        r#"
fn build() -> Result<Built, Failure> { Result.Ok(Built { value: present()? }) }
fn probe() -> Int {
    match build() { Result.Ok(built) => accept(built.value), Result.Err(_) => -1 }
}
"#,
        7,
    );
}

#[test]
fn a_successful_nullable_field_is_distinct_from_a_missing_field() {
    value(
        r#"
fn build() -> Result<Built, Failure> { Result.Ok(Built { value: nullable()? }) }
fn probe() -> Int {
    match build() { Result.Ok(built) => accept(built.value), Result.Err(_) => -1 }
}
"#,
        9,
    );
}

#[test]
fn an_annotated_initializer_cannot_change_the_residual_target() {
    value(
        r#"
fn build() -> Result<Built, Failure> {
    let value: Maybe<Int> = missing()?;
    Result.Ok(Built { value })
}
fn probe() -> Int { match build() { Result.Ok(_) => 0, Result.Err(error) => error.code } }
"#,
        41,
    );
}

#[test]
fn a_call_argument_hint_cannot_change_the_residual_target() {
    value(
        r#"
fn build() -> Result<Int, Failure> { Result.Ok(accept(missing()?)) }
fn probe() -> Int { match build() { Result.Ok(_) => 0, Result.Err(error) => error.code } }
"#,
        41,
    );
}

#[test]
fn nested_record_hints_cannot_change_the_residual_target() {
    value(
        r#"
fn build() -> Result<Nested, Failure> {
    Result.Ok(Nested { built: Built { value: missing()? } })
}
fn probe() -> Int { match build() { Result.Ok(_) => 0, Result.Err(error) => error.code } }
"#,
        41,
    );
}

#[test]
fn a_maybe_function_still_converts_a_result_error_to_none() {
    value(
        r#"
fn build() -> Maybe<ResultField> { Maybe.Some(ResultField { value: missing_result()? }) }
fn probe() -> Int { match build() { Maybe.Some(_) => 0, Maybe.None => 29 } }
"#,
        29,
    );
}

#[test]
fn declared_error_conversion_keeps_the_full_function_return_type() {
    value(
        r#"
fn build() -> Result<Built, ConvertedFailure> { Result.Ok(Built { value: missing()? }) }
fn probe() -> Int { match build() { Result.Ok(_) => 0, Result.Err(error) => error.code } }
"#,
        141,
    );
}

#[test]
fn a_plain_try_block_catches_the_field_error_locally() {
    value(
        r#"
fn probe() -> Int {
    let outcome = try { Built { value: missing()? } };
    match outcome { Result.Ok(_) => 0, Result.Err(error) => error.code }
}
"#,
        41,
    );
}

#[test]
fn a_recover_block_catches_the_field_error_locally() {
    value(
        r#"
fn probe() -> Int {
    let built = try { Built { value: missing()? } }
        recover { error => Built { value: Maybe.Some(error.code) } };
    accept(built.value)
}
"#,
        41,
    );
}

#[test]
fn an_explicit_closure_owns_its_result_target() {
    value(
        r#"
fn probe() -> Int {
    let work = || -> Result<Built, Failure> { Result.Ok(Built { value: missing()? }) };
    match work() { Result.Ok(_) => 0, Result.Err(error) => error.code }
}
"#,
        41,
    );
}

#[test]
fn an_inferred_closure_owns_its_result_target() {
    value(
        r#"
fn probe() -> Int {
    let work = || { Result.Ok(Built { value: missing()? }) };
    match work() { Result.Ok(_) => 0, Result.Err(error) => error.code }
}
"#,
        41,
    );
}

#[test]
fn a_callback_annotation_keeps_its_own_return_target() {
    value(
        r#"
fn probe() -> Int {
    let work: fn() -> Result<Built, Failure> = || { Result.Ok(Built { value: missing()? }) };
    match work() { Result.Ok(_) => 0, Result.Err(error) => error.code }
}
"#,
        41,
    );
}

#[test]
fn callable_try_boundary_does_not_inherit_the_enclosing_handler() {
    value(
        r#"
fn probe() -> Int {
    try {
        let work = || -> Result<Int, Failure> { let value = missing()?; Result.Ok(7) };
        match work() { Result.Ok(_) => 0, Result.Err(error) => error.code }
    } recover { _ => 99 }
}
"#,
        41,
    );
}

#[test]
fn callable_try_boundary_can_be_called_after_the_outer_handler_ends() {
    value(
        r#"
fn probe() -> Int {
    let work = try {
        || -> Result<Int, Failure> { let value = missing()?; Result.Ok(7) }
    } recover { _ => || -> Result<Int, Failure> { Result.Ok(0) } };
    match work() { Result.Ok(_) => 0, Result.Err(error) => error.code }
}
"#,
        41,
    );
}

#[test]
fn callable_try_boundary_retains_a_handler_inside_the_closure() {
    value(
        r#"
fn probe() -> Int {
    try {
        let work = || -> Result<Int, Failure> {
            let code = try { let value = missing()?; 0 } recover { error => error.code };
            Result.Ok(code + 1)
        };
        match work() { Result.Ok(code) => code, Result.Err(_) => 0 }
    } recover { _ => 99 }
}
"#,
        42,
    );
}

#[test]
fn callable_try_boundary_restores_the_enclosing_handler_after_compilation() {
    value(
        r#"
fn probe() -> Int {
    try {
        let work = || -> Result<Int, Failure> { Result.Ok(7) };
        let value = missing()?;
        0
    } recover { error => error.code + 2 }
}
"#,
        43,
    );
}

#[test]
fn nested_closure_compilation_restores_the_outer_residual_target() {
    value(
        r#"
fn build() -> Result<Built, Failure> {
    let work = || -> Maybe<Int> { let value = missing()?; Maybe.Some(7) };
    let marker = match work() { Maybe.None => 29, Maybe.Some(_) => 0 };
    if marker != 29 { return Result.Ok(Built { value: Maybe.None }); }
    let value = missing()?;
    Result.Ok(Built { value })
}
fn probe() -> Int { match build() { Result.Ok(_) => 0, Result.Err(error) => error.code } }
"#,
        41,
    );
}

#[test]
fn a_result_function_still_converts_a_maybe_residual() {
    value(
        r#"
fn build() -> Result<Int, Failure> { Result.Ok(absent()?) }
fn probe() -> Int { match build() { Result.Ok(_) => 0, Result.Err(error) => error.code } }
"#,
        83,
    );
}

#[test]
fn contextual_residual_uses_full_let_callable_signature() {
    value(
        r#"
fn probe() -> Int {
    let work: fn() -> Result<Built, ConvertedFailure> = || {
        Result.Ok(Built { value: missing()? })
    };
    match work() { Result.Ok(_) => 0, Result.Err(error) => error.code }
}
"#,
        141,
    );
}

#[test]
fn contextual_residual_uses_full_callback_parameter_signature() {
    value(
        r#"
fn invoke(work: fn() -> Result<Built, ConvertedFailure>) -> Result<Built, ConvertedFailure> {
    work()
}
fn probe() -> Int {
    match invoke(|| { Result.Ok(Built { value: missing()? }) }) {
        Result.Ok(_) => 0, Result.Err(error) => error.code,
    }
}
"#,
        141,
    );
}

#[test]
fn contextual_residual_uses_full_callable_record_field_signature() {
    value(
        r#"
type Task is { work: fn() -> Result<Built, ConvertedFailure> };
fn probe() -> Int {
    let task = Task { work: || { Result.Ok(Built { value: missing()? }) } };
    match (task.work)() { Result.Ok(_) => 0, Result.Err(error) => error.code }
}
"#,
        141,
    );
}

#[test]
fn contextual_residual_preserves_an_explicit_closure_error_conversion() {
    value(
        r#"
fn probe() -> Int {
    let work = || -> Result<Built, ConvertedFailure> {
        Result.Ok(Built { value: missing()? })
    };
    match work() { Result.Ok(_) => 0, Result.Err(error) => error.code }
}
"#,
        141,
    );
}

#[test]
fn contextual_residual_uses_full_static_callback_signature() {
    value(
        r#"
type Runner is { marker: Int };
implement Runner {
    fn invoke(work: fn() -> Result<Built, ConvertedFailure>) -> Result<Built, ConvertedFailure> {
        work()
    }
}
fn probe() -> Int {
    match Runner.invoke(|| { Result.Ok(Built { value: missing()? }) }) {
        Result.Ok(_) => 0, Result.Err(error) => error.code,
    }
}
"#,
        141,
    );
}

#[test]
fn contextual_residual_uses_full_instance_callback_signature() {
    value(
        r#"
type Runner is { marker: Int };
implement Runner {
    fn invoke(&self, work: fn() -> Result<Built, ConvertedFailure>) -> Result<Built, ConvertedFailure> {
        work()
    }
}
fn probe() -> Int {
    let runner = Runner { marker: 0 };
    match runner.invoke(|| { Result.Ok(Built { value: missing()? }) }) {
        Result.Ok(_) => 0, Result.Err(error) => error.code,
    }
}
"#,
        141,
    );
}

#[test]
fn contextual_residual_uses_full_callable_assignment_signature() {
    value(
        r#"
fn probe() -> Int {
    let mut work: fn() -> Result<Built, ConvertedFailure> = || {
        Result.Ok(Built { value: Maybe.None })
    };
    work = || { Result.Ok(Built { value: missing()? }) };
    match work() { Result.Ok(_) => 0, Result.Err(error) => error.code }
}
"#,
        141,
    );
}

#[test]
fn contextual_residual_keeps_sibling_callback_signatures_independent() {
    value(
        r#"
fn combine(first: fn() -> Result<Built, ConvertedFailure>, second: fn() -> Maybe<Int>) -> Int {
    let left = match first() { Result.Ok(_) => 0, Result.Err(error) => error.code };
    let right = match second() { Maybe.Some(_) => 0, Maybe.None => 29 };
    left + right
}
fn probe() -> Int {
    combine(
        || { Result.Ok(Built { value: missing()? }) },
        || { let value = missing()?; Maybe.Some(0) },
    )
}
"#,
        170,
    );
}
