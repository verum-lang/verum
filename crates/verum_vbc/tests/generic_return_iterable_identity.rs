#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

fn run(source: &str) -> i64 {
    run_with_result_type(source, None)
}

fn run_with_result_type(source: &str, result_type: Option<&str>) -> i64 {
    // The public report snapshots the previous function when the next
    // function starts; flush probe's bindings before reading that report.
    let source = format!("{source}\nfn after_probe() {{}}\n");
    let ast = Parser::new(&source).parse_module().expect("parse");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("generic_return_pins"));
    let module = codegen.compile_module(&ast).expect("compile");
    if let Some(result_type) = result_type {
        assert_eq!(
            codegen
                .variable_type_names()
                .get("result")
                .map(String::as_str),
            Some(result_type),
            "the public inferred-type report must preserve the nominal identity"
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
fn direct_generic_return_preserves_the_iterable_type() {
    assert_eq!(
        run(r#"
fn identity<Element>(value: Element) -> Element { value }
fn probe() -> Int {
    let values = identity([7, 11]);
    let mut total = 0;
    for value in values { total = total + value; }
    total
}
"#),
        18
    );
}

#[test]
fn sibling_blocks_preserve_generic_list_results() {
    assert_eq!(
        run(r#"
fn identity<T>(value: T) -> T { value }
fn probe() -> Int {
    let first = { let list = identity([7, 11]); list };
    let mut total = 0;
    for value in first { total = total + value; }
    let second = { let list = identity([13, 17]); list };
    for value in second { total = total + value; }
    total
}
"#),
        48
    );
}

#[test]
fn generic_return_preserves_a_record_layout() {
    assert_eq!(
        run(r#"
type Answer is { padding: Int, answer: Int };
fn identity<Element>(value: Element) -> Element { value }
fn probe() -> Int {
    let result = identity(Answer { padding: 99, answer: 7 });
    result.answer
}
"#),
        7
    );
}

#[test]
fn sibling_blocks_use_their_own_result_binding() {
    assert_eq!(
        run(r#"
type First is { answer: Int, padding: Int };
type Second is { padding: Int, answer: Int };
fn probe() -> Int {
    let first = { let item = First { answer: 3, padding: 90 }; item };
    let second = { let item = Second { padding: 99, answer: 7 }; item };
    first.answer + second.answer
}
"#),
        10
    );
}

#[test]
fn leaving_a_block_restores_the_outer_binding_type() {
    assert_eq!(
        run(r#"
type First is { answer: Int, padding: Int };
type Second is { padding: Int, answer: Int };
fn probe() -> Int {
    let item = First { answer: 7, padding: 99 };
    { let item = Second { padding: 41, answer: 11 }; item.answer; }
    item.answer
}
"#),
        7
    );
}

#[test]
fn leaving_a_block_preserves_refinements_of_outer_bindings() {
    for push in [
        "values.push(Actual { padding: 99, answer: 7 });",
        "{ values.push(Actual { padding: 99, answer: 7 }); }",
    ] {
        let source = format!(
            r#"
type Decoy is {{ answer: Int, padding: Int, extra: Int }};
type Actual is {{ padding: Int, answer: Int }};
fn probe() -> Int {{
    let mut values = List.new();
    {push}
    values[0].answer
}}
fn capture_type_report() {{}}
"#
        );
        assert_eq!(run(&source), 7, "{push}");
        let ast = Parser::new(&source).parse_module().expect("parse");
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("block_refinement"));
        codegen.compile_module(&ast).expect("compile");
        assert_eq!(
            codegen
                .variable_type_names()
                .get("values")
                .map(String::as_str),
            Some("List<Actual>"),
            "{push}: {:?}",
            codegen.variable_type_names()
        );
    }
}
#[test]
fn a_dereferenced_generic_guard_passes_its_target_type_to_a_generic_call() {
    assert_eq!(
        run(r#"
type Guard<Element> is { value: Element };
implement<Element> Guard<Element> {
    fn deref(&self) -> &Element { &self.value }
}
fn identity<Value>(value: Value) -> Value { value }
fn probe() -> Int {
    let guard: Guard<List<Int>> = Guard { value: [7, 11] };
    let values = identity(*guard);
    let mut total = 0;
    for value in values { total = total + value; }
    total
}
"#),
        18
    );
}

#[test]
fn generic_replace_drains_sibling_guard_blocks_without_losing_list_identity() {
    assert_eq!(
        run(r#"
type Guard<Element> is { value: Element };
implement<Element> Guard<Element> {
    fn deref(&self) -> &Element { &self.value }
    fn deref_mut(&mut self) -> &mut Element { &mut self.value }
}
fn replace<Value>(dest: &mut Value, src: Value) -> Value {
    @intrinsic("replace", dest, src)
}
fn probe() -> Int {
    let mut first_guard: Guard<List<Int>> = Guard { value: [7, 11] };
    let first = { let list = replace(&mut *first_guard, List.new()); list };
    let mut total = 0;
    for value in first { total = total + value; }
    let mut second_guard: Guard<List<Int>> = Guard { value: [13, 17] };
    let second = { let list = replace(&mut *second_guard, List.new()); list };
    for value in second { total = total + value; }
    total
}
"#),
        48
    );
}

#[test]
fn a_short_uppercase_nominal_is_a_concrete_generic_argument() {
    assert_eq!(
        run_with_result_type(
            r#"
type Decoy is { answer: Int, padding: Int, extra: Int };
type AB is { padding: Int, answer: Int };
fn identity<Element>(value: Element) -> Element { value }
fn probe() -> Int {
    let result = identity(AB { padding: 99, answer: 7 });
    result.answer
}
"#,
            Some("AB")
        ),
        7
    );
}

#[test]
fn a_container_binds_a_short_uppercase_nominal_argument() {
    assert_eq!(
        run_with_result_type(
            r#"
type Decoy is { answer: Int, padding: Int, extra: Int };
type AB is { padding: Int, answer: Int };
fn first<Element>(values: List<Element>) -> Element { values[0] }
fn probe() -> Int {
    let result = first([AB { padding: 99, answer: 7 }]);
    result.answer
}
"#,
            Some("AB")
        ),
        7
    );
}
