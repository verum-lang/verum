#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::bytecode::decode_instruction;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::interpreter::Interpreter;

// The local method isolates dispatch from the stdlib's case-folding algorithm.
// Its name and reference signature match the public Text method used by Weft.
const ENV: &str = r#"
implement Text {
    fn eq_ignore_case(&self, other: &Text) -> Bool { *self == *other }
}
type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
type Guard<T> is { marker: Int, value: T };
implement<T> Deref for Guard<T> {
    type Target = T;
    fn deref(&self) -> &T { &self.value }
}
implement<T> Guard<T> { fn own(&self) -> Int { self.marker } }
type Cell is { padding: Int, value: Int };
implement Cell {
    fn read(&self) -> Int { self.value }
    fn own(&self) -> Int { 99 }
}
type GuardHolder is { guard: Guard<Cell> };
implement GuardHolder {
    fn get(&self) -> Maybe<&Guard<Cell>> { Maybe.Some(&self.guard) }
}
"#;

fn check(body: &str, expected: Option<i64>) -> List<Text> {
    let ast = Parser::new(&format!("{ENV}\n{body}"))
        .parse_module()
        .expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("reference_receiver"))
        .compile_module(&ast)
        .expect("compile");
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe");
    let entry_id = entry.id;
    let mut operations = List::new();
    for function in &module.functions {
        let end = (function.bytecode_offset + function.bytecode_length) as usize;
        let mut offset = function.bytecode_offset as usize;
        while offset < end {
            match decode_instruction(&module.bytecode, &mut offset).expect("instruction") {
                Instruction::CallM { method_id, .. } => operations.push(Text::from(
                    module
                        .get_string(verum_vbc::types::StringId(method_id))
                        .unwrap(),
                )),
                Instruction::Call { func_id, .. }
                | Instruction::TailCall { func_id, .. }
                | Instruction::CallG { func_id, .. } => operations.push(Text::from(
                    module
                        .get_string(module.functions[func_id as usize].name)
                        .unwrap(),
                )),
                _ => {}
            }
        }
    }
    assert!(
        operations.iter().all(|name| !name.starts_with("&")),
        "{operations:?}"
    );
    if let Some(expected) = expected {
        let value = Interpreter::new(Arc::new(module))
            .execute_function(entry_id)
            .expect("execute");
        assert!(value.is_int(), "expected Int, got {value:?}");
        assert_eq!(value.as_i64(), expected);
    }
    operations
}

fn assert_text_owner(operations: &List<Text>) {
    assert!(
        operations
            .iter()
            .any(|name| name.ends_with("Text.eq_ignore_case")),
        "{operations:?}"
    );
}

#[test]
fn direct_text_reference_keeps_the_pointee_method_owner() {
    assert_text_owner(&check(
        r#"
fn probe() -> Int {
    let text = "close";
    let value: &Text = &text;
    if value.eq_ignore_case(&"close") { 7 } else { -1 }
}
"#,
        Some(7),
    ));
    assert_text_owner(&check(
        "fn probe(value: &Text) -> Bool { value.eq_ignore_case(&\"close\") }",
        None,
    ));
}

#[test]
fn optional_text_reference_payload_dispatches_in_a_match_guard() {
    assert_text_owner(&check(
        r#"
fn inspect_header(input: Maybe<&Text>) -> Int {
    match input { Maybe.Some(value) if value.eq_ignore_case(&"close") => 7, _ => -1 }
}
fn probe() -> Int {
    let text = "close";
    inspect_header(Maybe.Some(&text))
}
"#,
        Some(7),
    ));
}

#[test]
fn parentheses_preserve_a_reference_payload_method_owner() {
    assert_text_owner(&check(
        r#"
fn probe() -> Int {
    let text = "close";
    let input: Maybe<&Text> = Maybe.Some(&text);
    match input { Maybe.Some(value) if (value).eq_ignore_case(&"close") => 7, _ => -1 }
}
"#,
        Some(7),
    ));
}

#[test]
fn reference_payload_keeps_wrapper_own_method_precedence() {
    let operations = check(
        r#"
fn probe() -> Int {
    let guard: Guard<Cell> = Guard { marker: 13, value: Cell { padding: 41, value: 7 } };
    let holder = GuardHolder { guard };
    let input = holder.get();
    match input { Maybe.Some(value) => value.own(), _ => -1 }
}
"#,
        Some(13),
    );
    assert!(
        operations.iter().any(|name| name.ends_with("Guard.own")),
        "{operations:?}"
    );
    assert!(
        !operations
            .iter()
            .any(|name| name.ends_with(".deref") || name.ends_with("Cell.own")),
        "{operations:?}"
    );
}

#[test]
fn reference_payload_forwards_only_when_the_wrapper_lacks_the_method() {
    let operations = check(
        r#"
fn probe() -> Int {
    let guard: Guard<Cell> = Guard { marker: 13, value: Cell { padding: 41, value: 7 } };
    let holder = GuardHolder { guard };
    let input = holder.get();
    match input { Maybe.Some(value) => value.read(), _ => -1 }
}
"#,
        Some(7),
    );
    assert!(
        operations.iter().any(|name| name.ends_with("Guard.deref")),
        "{operations:?}"
    );
    assert!(
        operations.iter().any(|name| name.ends_with("Cell.read")),
        "{operations:?}"
    );
}

#[test]
fn method_return_reference_payload_keeps_text_identity() {
    assert_text_owner(&check(
        r#"
type Headers is { connection: Text };
implement Headers {
    fn get(&self, _key: &Text) -> Maybe<&Text> { Maybe.Some(&self.connection) }
}
fn probe() -> Int {
    let headers = Headers { connection: "close" };
    let conn_v = headers.get(&"connection");
    match conn_v { Maybe.Some(v) if v.eq_ignore_case(&"close") => 7, _ => -1 }
}
"#,
        Some(7),
    ));
}

#[test]
fn explicit_reference_deref_in_a_payload_still_keeps_wrapper_methods() {
    let operations = check(
        r#"
fn probe() -> Int {
    let guard: Guard<Cell> = Guard { marker: 13, value: Cell { padding: 41, value: 7 } };
    let holder = GuardHolder { guard };
    let input = holder.get();
    match input { Maybe.Some(value) => (*(value)).own(), _ => -1 }
}
"#,
        Some(13),
    );
    assert!(
        operations.iter().any(|name| name.ends_with("Guard.own")),
        "{operations:?}"
    );
    assert!(
        !operations
            .iter()
            .any(|name| name.ends_with(".deref") || name.ends_with("Cell.own")),
        "{operations:?}"
    );
}
