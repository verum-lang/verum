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

#[test]
fn record_text_fields_are_equal_across_inline_and_heap_storage() {
    let mut module = VbcModule::default();
    module.types.push(TypeDescriptor {
        id: TypeId(9001),
        kind: TypeKind::Record,
        fields: [FieldDescriptor::default()].into_iter().collect(),
        ..TypeDescriptor::default()
    });
    let mut state = InterpreterState::new(Arc::new(module));
    let inline = Value::from_small_string("1.0.0").expect("inline version");
    let text = state.heap.alloc_text(b"1.0.0").expect("heap version");
    let heap = Value::from_ptr(text.as_ptr() as *mut u8);
    let first = record(&mut state, TypeId(9001), &[inline]);
    let equal = record(&mut state, TypeId(9001), &[heap]);
    assert!(deep_value_eq(&first, &equal, &state));
    assert_eq!(value_hash(first, &state), value_hash(equal, &state));
    assert!(value_eq(first, equal, &state));
    assert!(value_eq(equal, first, &state));
}

#[test]
fn nested_record_keys_survive_map_growth() {
    assert_eq!(
        run(r#"
type Part is { number: Int };
type Key is { part: Part };
fn key(number: Int) -> Key { Key { part: Part { number: number } } }
fn probe() -> Int {
    let mut values: Map<Key, Int> = Map.new();
    let mut i = 0;
    while i < 48 { values.insert(key(i), i + 100); i = i + 1; }
    let mut found = 0;
    i = 0;
    while i < 48 {
        match values.get(key(i)) {
            Maybe.Some(value) => { if value == i + 100 { found = found + 1; } },
            Maybe.None => {},
        }
        i = i + 1;
    }
    found
}
"#),
        48
    );
}

#[test]
fn set_record_keys_deduplicate_and_remove_by_value() {
    assert_eq!(
        run(&format!(
            r#"{COORDINATE}
fn probe() -> Int {{
    let mut keys: Set<ReleaseCoordinate> = Set.new();
    keys.insert(coordinate("@acme/tool", "1.0.0"));
    keys.insert(coordinate("@acme/tool", "1.0.0"));
    keys.insert(coordinate("@acme/tool", "2.0.0"));
    let mut flags = 0;
    if keys.len() == 2 {{ flags = flags + 1; }}
    if keys.contains(coordinate("@acme/tool", "1.0.0")) {{ flags = flags + 2; }}
    keys.remove(coordinate("@acme/tool", "1.0.0"));
    if !keys.contains(coordinate("@acme/tool", "1.0.0")) {{ flags = flags + 4; }}
    if keys.contains(coordinate("@acme/tool", "2.0.0")) {{ flags = flags + 8; }}
    flags
}}
"#
        )),
        15
    );
}

#[test]
fn record_raw_pointer_fields_keep_identity_despite_header_like_bytes() {
    let mut module = VbcModule::default();
    module.types.push(TypeDescriptor {
        id: TypeId(9001),
        kind: TypeKind::Record,
        fields: [FieldDescriptor::default()].into_iter().collect(),
        ..TypeDescriptor::default()
    });
    let mut state = InterpreterState::new(Arc::new(module));
    // Raw buffers may legally contain any bytes, including an apparent Text
    // header. These are aligned initialized bytes, but not tracked objects.
    fn raw_text_shaped_bytes() -> [u64; 6] {
        let mut bytes = [0u64; 6];
        let ptr = bytes.as_mut_ptr().cast::<u8>();
        // SAFETY: the aligned buffer holds a 24-byte header plus three Values.
        unsafe {
            ptr.cast::<heap::ObjectHeader>()
                .write(heap::ObjectHeader::new(TypeId::TEXT, 0, 24));
            let slots = ptr.add(heap::OBJECT_HEADER_SIZE).cast::<Value>();
            for i in 0..3 {
                slots.add(i).write(Value::from_i64(0));
            }
        }
        bytes
    }
    let mut bytes_a = raw_text_shaped_bytes();
    let mut bytes_b = raw_text_shaped_bytes();
    let raw_a = Value::from_ptr(bytes_a.as_mut_ptr().cast::<u8>());
    let raw_b = Value::from_ptr(bytes_b.as_mut_ptr().cast::<u8>());
    let first = record(&mut state, TypeId(9001), &[raw_a]);
    let same_pointer = record(&mut state, TypeId(9001), &[raw_a]);
    let other_pointer = record(&mut state, TypeId(9001), &[raw_b]);
    assert!(
        !value_eq(first, other_pointer, &state),
        "raw payload bytes are not type evidence"
    );
    assert!(value_eq(first, same_pointer, &state));
    let hash = value_hash(first, &state);
    assert_eq!(hash, value_hash(same_pointer, &state));
    bytes_a[0] = 0; // Changing raw contents cannot change a pointer-valued key.
    assert_eq!(bytes_a[0], 0);
    assert_eq!(hash, value_hash(first, &state));
    assert!(value_eq(first, same_pointer, &state));
}

#[test]
fn native_shared_descriptor_does_not_make_refcount_a_key_field() {
    let mut module = VbcModule::default();
    module.types.push(TypeDescriptor {
        id: TypeId(9001),
        kind: TypeKind::Record,
        fields: [FieldDescriptor::default()].into_iter().collect(),
        ..TypeDescriptor::default()
    });
    // An archive can contain the stdlib Shared declaration, while the
    // interpreter's native carrier still has [refcount, inner] storage.
    module.types.push(TypeDescriptor {
        id: TypeId::SHARED,
        kind: TypeKind::Record,
        fields: [
            FieldDescriptor::default(),
            FieldDescriptor::default(),
            FieldDescriptor::default(),
        ]
        .into_iter()
        .collect(),
        ..TypeDescriptor::default()
    });
    let mut state = InterpreterState::new(Arc::new(module));
    let shared = record(
        &mut state,
        TypeId::SHARED,
        &[Value::from_i64(1), Value::from_i64(42)],
    );
    let key = record(&mut state, TypeId(9001), &[shared]);
    let same = record(&mut state, TypeId(9001), &[shared]);
    let hash = value_hash(key, &state);
    let alias = super::memory_collections::value_copy(&mut state, shared).expect("Shared copy");
    assert_eq!(shared.to_bits(), alias.to_bits());
    let refcount = unsafe {
        *shared
            .as_ptr::<u8>()
            .add(heap::OBJECT_HEADER_SIZE)
            .cast::<Value>()
    };
    assert_eq!(refcount.as_i64(), 2, "the control must change the refcount");
    assert!(value_eq(key, same, &state));
    assert_eq!(
        hash,
        value_hash(key, &state),
        "native bookkeeping is not a record field"
    );
    assert_eq!(hash, value_hash(same, &state));
}

#[test]
fn native_range_and_ordering_keep_their_canonical_key_representations() {
    let mut module = VbcModule::default();
    module.types.push(TypeDescriptor {
        id: TypeId::RANGE,
        kind: TypeKind::Record,
        fields: [FieldDescriptor::default(), FieldDescriptor::default()]
            .into_iter()
            .collect(),
        ..TypeDescriptor::default()
    });
    let mut state = InterpreterState::new(Arc::new(module));
    // RANGE used to be mistaken for Ordering by a stale literal 517. Its
    // Value-encoded bounds must never be read as a variant tag/field count.
    let range = record(
        &mut state,
        TypeId::RANGE,
        &[Value::from_i64(1), Value::from_i64(42)],
    );
    let _ = value_hash(range, &state);
    fn ordering(state: &mut InterpreterState, tag: u32) -> Value {
        let object = state.heap.alloc(TypeId::ORDERING, 8).expect("Ordering");
        let ptr = object.as_ptr() as *mut u8;
        // SAFETY: a nullary typed variant has a tag/count pair and no payload.
        unsafe {
            let pair = ptr.add(heap::OBJECT_HEADER_SIZE).cast::<u32>();
            pair.write(tag);
            pair.add(1).write(0);
        }
        Value::from_ptr(ptr)
    }
    let equal_a = ordering(&mut state, 1);
    let equal_b = ordering(&mut state, 1);
    let less = ordering(&mut state, 0);
    assert!(value_eq(equal_a, equal_b, &state));
    assert_eq!(value_hash(equal_a, &state), value_hash(equal_b, &state));
    assert!(!value_eq(equal_a, less, &state));
}
