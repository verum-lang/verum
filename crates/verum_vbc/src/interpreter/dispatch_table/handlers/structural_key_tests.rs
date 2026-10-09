//! Source -> VBC wire -> interpreter controls for structural collection keys.
//! These bypass the type checker, so they do not establish derive acceptance.

use super::memory_collections::{value_eq, value_hash};
use super::string_helpers::deep_value_eq;
use crate::codegen::VbcCodegen;
use crate::interpreter::{Interpreter, heap, state::InterpreterState};
use crate::module::VbcModule;
use crate::types::{FieldDescriptor, TypeDescriptor, TypeId, TypeKind};
use crate::value::Value;
use std::sync::Arc;
use verum_common::List;
use verum_fast_parser::Parser;

const COORDINATE: &str = r#"
@derive(Eq, Hash, Clone)
type ReleaseCoordinate is { name: Text, version: Text };
fn coordinate(name: Text, version: Text) -> ReleaseCoordinate {
    ReleaseCoordinate { name: name, version: version }
}
fn read(values: &Map<ReleaseCoordinate, Int>, name: Text, version: Text) -> Int {
    match values.get(coordinate(name, version)) {
        Maybe.Some(value) => value,
        Maybe.None => -1,
    }
}
"#;

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut codegen = VbcCodegen::new();
    codegen.register_stdlib_intrinsics();
    let module = codegen.compile_module(&ast).expect("source VBC");
    let bytes = crate::serialize::serialize_module(&module).expect("wire writer");
    crate::deserialize::deserialize_module(&bytes).expect("wire reader")
}

fn run_module(module: VbcModule, args: &[Value]) -> i64 {
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe function")
        .id;
    let result = Interpreter::new(Arc::new(module))
        .execute_function_with_args(entry, args)
        .expect("execute probe");
    assert!(result.is_int(), "probe must return Int: {result:?}");
    result.as_i64()
}

fn run(source: &str) -> i64 {
    run_module(compile(source), &[])
}

#[test]
fn separately_allocated_record_keys_find_the_existing_entry() {
    assert_eq!(
        run(&format!(
            r#"{COORDINATE}
fn probe() -> Int {{
    let mut values: Map<ReleaseCoordinate, Int> = Map.new();
    values.insert(coordinate("@acme/tool", "1.0.0"), 41);
    read(&values, "@acme/tool", "1.0.0")
}}
"#
        )),
        41
    );
}

#[test]
fn record_key_components_distinguish_names_and_versions() {
    assert_eq!(
        run(&format!(
            r#"{COORDINATE}
fn probe() -> Int {{
    let mut values: Map<ReleaseCoordinate, Int> = Map.new();
    values.insert(coordinate("@acme/tool", "1.0.0"), 11);
    values.insert(coordinate("@acme/tool", "2.0.0"), 22);
    values.insert(coordinate("@other/tool", "1.0.0"), 33);
    let mut flags = 0;
    if read(&values, "@acme/tool", "1.0.0") == 11 {{ flags = flags + 1; }}
    if read(&values, "@acme/tool", "2.0.0") == 22 {{ flags = flags + 2; }}
    if read(&values, "@other/tool", "1.0.0") == 33 {{ flags = flags + 4; }}
    if read(&values, "@absent/tool", "1.0.0") == -1 {{ flags = flags + 8; }}
    if read(&values, "@acme/tool", "3.0.0") == -1 {{ flags = flags + 16; }}
    flags
}}
"#
        )),
        31
    );
}

#[test]
fn equal_record_key_replaces_one_entry_and_removes_by_value() {
    assert_eq!(
        run(&format!(
            r#"{COORDINATE}
fn probe() -> Int {{
    let mut values: Map<ReleaseCoordinate, Int> = Map.new();
    values.insert(coordinate("@acme/tool", "1.0.0"), 11);
    values.insert(coordinate("@acme/tool", "1.0.0"), 42);
    let mut flags = 0;
    if values.len() == 1 {{ flags = flags + 1; }}
    if read(&values, "@acme/tool", "1.0.0") == 42 {{ flags = flags + 2; }}
    values.remove(coordinate("@acme/tool", "1.0.0"));
    if values.len() == 0 {{ flags = flags + 4; }}
    if read(&values, "@acme/tool", "1.0.0") == -1 {{ flags = flags + 8; }}
    flags
}}
"#
        )),
        15
    );
}

#[test]
fn text_keys_remain_a_control() {
    assert_eq!(
        run(r#"
fn probe() -> Int {
    let mut values: Map<Text, Int> = Map.new();
    values.insert("@acme/tool", 41);
    match values.get("@acme/tool") { Maybe.Some(value) => value, Maybe.None => -1 }
}
"#),
        41
    );
}

fn record(state: &mut InterpreterState, id: TypeId, fields: &[Value]) -> Value {
    let object = state
        .heap
        .alloc(id, std::mem::size_of_val(fields))
        .expect("record");
    let ptr = object.as_ptr() as *mut u8;
    // SAFETY: this tracked object has exactly fields.len() initialized Value slots.
    unsafe {
        std::ptr::copy_nonoverlapping(
            fields.as_ptr(),
            ptr.add(heap::OBJECT_HEADER_SIZE).cast(),
            fields.len(),
        );
    }
    Value::from_ptr(ptr)
}

#[test]
fn record_type_identity_and_declared_fields_are_shared_with_equality() {
    let mut module = VbcModule::default();
    for id in [TypeId(9001), TypeId(9002)] {
        module.types.push(TypeDescriptor {
            id,
            kind: TypeKind::Record,
            fields: [FieldDescriptor::default(), FieldDescriptor::default()]
                .into_iter()
                .collect(),
            ..TypeDescriptor::default()
        });
    }
    let mut state = InterpreterState::new(Arc::new(module));
    let first = record(
        &mut state,
        TypeId(9001),
        &[
            Value::from_i64(10),
            Value::from_i64(20),
            Value::from_i64(77),
        ],
    );
    let equal = record(
        &mut state,
        TypeId(9001),
        &[
            Value::from_i64(10),
            Value::from_i64(20),
            Value::from_i64(88),
        ],
    );
    let other_type = record(
        &mut state,
        TypeId(9002),
        &[
            Value::from_i64(10),
            Value::from_i64(20),
            Value::from_i64(77),
        ],
    );
    assert_ne!(first.to_bits(), equal.to_bits(), "separate allocations");
    assert!(
        deep_value_eq(&first, &equal, &state),
        "padding is not a declared field"
    );
    assert!(
        !deep_value_eq(&first, &other_type, &state),
        "nominal record identity"
    );
    assert!(value_eq(first, equal, &state));
    assert_eq!(value_hash(first, &state), value_hash(equal, &state));
    assert!(!value_eq(first, other_type, &state));
}

#[test]
fn records_survive_bucket_collisions_and_removal() {
    let module = compile(
        r#"
type Key is { number: Int };
fn key(number: Int) -> Key { Key { number: number } }
fn read(values: &Map<Key, Int>, number: Int) -> Int {
    match values.get(key(number)) { Maybe.Some(value) => value, Maybe.None => -1 }
}
fn probe(left: Int, right: Int) -> Int {
    let mut values: Map<Key, Int> = Map.new();
    values.insert(key(left), 17);
    values.insert(key(right), 29);
    let mut flags = 0;
    if read(&values, left) == 17 { flags = flags + 1; }
    if read(&values, right) == 29 { flags = flags + 2; }
    values.remove(key(left));
    if read(&values, left) == -1 { flags = flags + 4; }
    if read(&values, right) == 29 { flags = flags + 8; }
    flags
}
"#,
    );
    let id = module
        .types
        .iter()
        .find(|t| module.get_string(t.name) == Some("Key"))
        .expect("Key descriptor")
        .id;
    let mut state = InterpreterState::new(Arc::new(module.clone()));
    let capacity = verum_common::layout::DEFAULT_COLLECTION_CAPACITY as usize;
    let mut buckets: List<Option<i64>> = std::iter::repeat_n(None, capacity).collect();
    let mut collision = None;
    for n in 0..=capacity as i64 {
        let key = record(&mut state, id, &[Value::from_i64(n)]);
        let slot = value_hash(key, &state) % capacity;
        if let Some(previous) = buckets[slot] {
            collision = Some((previous, n));
            break;
        }
        buckets[slot] = Some(n);
    }
    let (left, right) = collision.expect("pigeonhole collision");
    assert_ne!(left, right);
    assert_eq!(
        run_module(module, &[Value::from_i64(left), Value::from_i64(right)]),
        15
    );
}
