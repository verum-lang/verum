//! T1196: mutable Map access must alias the value slot owned by its runtime carrier.
//!
//! Compile the complete production Map module and its caller without filtering or
//! replacing the Slot/get_mut declarations. Primitive bodies are also compiled,
//! so collection helpers execute the real Clone implementations. The mount
//! loader supplies other dependency declarations. Callers use real map literals (NewMap) to select the runtime
//! carrier; source-only Map.new takes a different route from the ordinary
//! archive compiler. No production body is filtered or replaced. Each executed
//! case must reach its own scalar oracle. This is a
//! source/wire interpreter gate, not a type-checker, AOT or lifetime-verifier gate.
#![cfg(feature = "codegen")]

use std::sync::{Arc, OnceLock};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::bytecode::decode_instructions;
use verum_vbc::codegen::{ItemFailurePolicy, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::VbcModule;

const PRODUCTION_MAP: &str = include_str!("../../../core/collections/map.vr");
const PRODUCTION_PRIMITIVES: &str = include_str!("../../../core/base/primitives.vr");
const CALLER: &str = r#"
module mutable_map_values;
mount core.collections.map.Map;

type Cell is { padding: Int, value: Int };
type Options is { version: Int, features: Maybe<Cell>, optional: Maybe<Bool> };
type Dependency is Requirement(Int) | Detailed(Options);
type Holder is { padding: Int, values: Map<Text, Int>, key: Text };

fn read_int(values: &Map<Text, Int>, key: Text) -> Int {
    match values.get(key) {
        Maybe.Some(value) => value,
        Maybe.None => -1000,
    }
}

fn replace(values: &mut Map<Text, Int>, key: &Text, replacement: Int) -> Int {
    match values.get_mut(key) {
        Maybe.Some(value) => {
            let previous = *value;
            *value = replacement;
            previous
        },
        Maybe.None => -2000,
    }
}

fn read_dependency(values: &Map<Text, Dependency>) -> Int {
    match values.get(&"codec") {
        Maybe.Some(Dependency.Detailed(options)) => {
            assert(options.optional is Maybe.Some(false));
            match &options.features {
                Maybe.Some(cell) => options.version * 10000 + cell.padding * 100 + cell.value,
                _ => -2,
            }
        },
        Maybe.Some(Dependency.Requirement(value)) => value,
        _ => -1,
    }
}

fn read_control() -> Int {
    let mut values: Map<Text, Int> = { "codec": 7 };
    values.insert("other", 19);
    read_int(&values, "codec") * 100 + read_int(&values, "other")
}

fn direct_write_through() -> Int {
    let mut values: Map<Text, Int> = { "codec": 7 };
    values.insert("other", 19);
    match values.get_mut(&"codec") {
        Maybe.Some(value) => {
            assert_eq(*value, 7);
            *value = 43;
            assert_eq(*value, 43);
        },
        Maybe.None => panic("present key lost"),
    }
    assert_eq(values.len(), 2);
    read_int(&values, "codec") * 100 + read_int(&values, "other")
}

fn empty_and_missing_keys() -> Int {
    let mut values: Map<Text, Int> = { "removed": 0 };
    values.clear();
    assert(values.get_mut(&"absent") is Maybe.None);
    values.insert("codec", 7);
    assert(values.get_mut(&"absent") is Maybe.None);
    assert_eq(values.len(), 1);
    read_int(&values, "codec")
}

fn forwarded_mutable_receiver_and_key() -> Int {
    let mut values: Map<Text, Int> = { "codec": 7 };
    let key = "codec";
    assert_eq(replace(&mut values, &key, 43), 7);
    read_int(&values, key)
}

fn record_field_receiver_and_key() -> Int {
    let mut values: Map<Text, Int> = { "codec": 7 };
    let mut holder = Holder { padding: 991, values: values, key: "codec" };
    assert_eq(replace(&mut holder.values, &holder.key, 43), 7);
    assert_eq(holder.padding, 991);
    read_int(&holder.values, "codec")
}

fn whole_variant_replacement() -> Int {
    let mut values: Map<Text, Dependency> = { "codec": Dependency.Detailed(Options {
        version: 13,
        features: Maybe.Some(Cell { padding: 93, value: 7 }),
        optional: Maybe.Some(false),
    }) };
    assert_eq(read_dependency(&values), 139307);
    match values.get_mut(&"codec") {
        Maybe.Some(value) => *value = Dependency.Requirement(61),
        Maybe.None => panic("present variant key lost"),
    }
    read_dependency(&values)
}

fn nested_payload_mutation() -> Int {
    let mut values: Map<Text, Dependency> = { "codec": Dependency.Detailed(Options {
        version: 13,
        features: Maybe.Some(Cell { padding: 93, value: 7 }),
        optional: Maybe.Some(false),
    }) };
    assert_eq(read_dependency(&values), 139307);
    match values.get_mut(&"codec") {
        Maybe.Some(Dependency.Detailed(options)) => {
            options.version = 17;
            match &mut options.features {
                Maybe.Some(cell) => {
                    cell.padding = 37;
                    cell.value = 11;
                },
                _ => panic("expected features"),
            }
        },
        _ => panic("expected detailed dependency"),
    }
    read_dependency(&values)
}

fn collision_chain() -> Int {
    let mut values: Map<Int, Int> = { 1: 7, 17: 19 };
    match values.get_mut(&17) {
        Maybe.Some(value) => *value = 43,
        Maybe.None => panic("collision key lost"),
    }
    assert(values.get_mut(&33) is Maybe.None);
    let first = match values.get(1) { Maybe.Some(value) => value, _ => -1000 };
    let second = match values.get(17) { Maybe.Some(value) => value, _ => -2000 };
    assert_eq(values.len(), 2);
    first * 100 + second
}

fn shared_tag_scalar() -> Int {
    let mut values: Map<Int, Int> = { 10: 520 };
    match values.get_mut(&10) {
        Maybe.Some(value) => {
            assert_eq(*value, 520);
            *value = 731;
            assert_eq(*value, 731);
        },
        Maybe.None => panic("scalar key lost"),
    }
    match values.get(10) { Maybe.Some(value) => value, _ => -1 }
}

fn grow_after_borrow_ends() -> Int {
    let mut values: Map<Int, Int> = { 1: 7 };
    match values.get_mut(&1) {
        Maybe.Some(value) => *value = 43,
        Maybe.None => panic("initial key lost"),
    }
    for key in 2..66 { values.insert(key, key + 100); }
    assert_eq(values.len(), 65);
    assert_eq(match values.get(1) { Maybe.Some(value) => value, _ => -1 }, 43);
    assert_eq(match values.get(65) { Maybe.Some(value) => value, _ => -1 }, 165);
    match values.get_mut(&65) {
        Maybe.Some(value) => *value = 211,
        Maybe.None => panic("grown key lost"),
    }
    match values.get(65) { Maybe.Some(value) => value, _ => -1 }
}

fn key_and_value_iterators() -> Int {
    let values: Map<Int, Int> = { 1: 11, 17: 19 };
    let mut keys = values.keys();
    let first = match keys.next() { Maybe.Some(key) => key, _ => -1000 };
    let second = match keys.next() { Maybe.Some(key) => key, _ => -2000 };
    assert(keys.next() is Maybe.None);
    assert(keys.next() is Maybe.None);
    assert(first != second);
    let mut items = values.values();
    let left = match items.next() { Maybe.Some(item) => item, _ => -3000 };
    let right = match items.next() { Maybe.Some(item) => item, _ => -4000 };
    assert(items.next() is Maybe.None);
    assert(left != right);
    (first + second) * 100 + left + right
}

fn collected_keys_and_values() -> Int {
    let mut values: Map<Int, Int> = { 1: 11, 17: 19 };
    let keys = values.keys_list();
    let items = values.values_list();
    assert_eq(keys.len(), 2);
    assert_eq(items.len(), 2);
    assert_eq(keys[0] + keys[1], 18);
    assert_eq(items[0] + items[1], 30);
    values.clear();
    assert_eq(values.keys_list().len(), 0);
    assert_eq(values.values_list().len(), 0);
    let mut empty_keys = values.keys();
    let mut empty_values = values.values();
    assert(empty_keys.next() is Maybe.None);
    assert(empty_values.next() is Maybe.None);
    1830
}

fn record_values_and_fresh_empty_iterators() -> Int {
    let values: Map<Int, Cell> = {
        1: Cell { padding: 101, value: 7 },
        17: Cell { padding: 303, value: 19 },
    };
    let mut items = values.values();
    let first = match items.next() { Maybe.Some(value) => value, _ => panic("first value") };
    let second = match items.next() { Maybe.Some(value) => value, _ => panic("second value") };
    assert_eq(first.padding + second.padding, 404);
    assert_eq(first.value + second.value, 26);
    assert(items.next() is Maybe.None);
    let empty: Map<Int, Cell> = Map.new();
    let mut keys = empty.keys();
    let mut items = empty.values();
    assert(keys.next() is Maybe.None);
    assert(items.next() is Maybe.None);
    assert_eq(empty.keys_list().len(), 0);
    assert_eq(empty.values_list().len(), 0);
    40426
}

"#;

fn source() -> Arc<VbcModule> {
    static MODULE: OnceLock<Arc<VbcModule>> = OnceLock::new();
    MODULE
        .get_or_init(|| {
            let map = Parser::new(PRODUCTION_MAP)
                .parse_module()
                .expect("production Map grammar");
            let caller = Parser::new(CALLER).parse_module().expect("caller grammar");
            let mut codegen = VbcCodegen::new();
            codegen
                .compile_module_with_mounts(
                    &map,
                    concat!(env!("CARGO_MANIFEST_DIR"), "/../../core/collections/map.vr"),
                    concat!(env!("CARGO_MANIFEST_DIR"), "/../../core"),
                )
                .expect("complete production Map source with declared dependencies");
            let primitives = Parser::new(PRODUCTION_PRIMITIVES)
                .parse_module()
                .expect("production primitive implementations");
            codegen
                .collect_unit_declarations(&[&primitives])
                .expect("primitive declarations");
            codegen
                .compile_unit_items(&[&primitives], ItemFailurePolicy::Strict)
                .expect("complete primitive bodies, including Int.clone");
            codegen
                .collect_unit_declarations(&[&caller])
                .expect("caller declarations");
            codegen
                .compile_unit_items(&[&caller], ItemFailurePolicy::Strict)
                .expect("caller bodies");
            let mut module = codegen.finalize_module().expect("complete module");
            module.resolve_protocol_dispatch();
            assert_real_mutable_body(&module);
            assert_runtime_carrier_callers(&module);
            Arc::new(module)
        })
        .clone()
}

fn wire() -> Arc<VbcModule> {
    static MODULE: OnceLock<Arc<VbcModule>> = OnceLock::new();
    MODULE
        .get_or_init(|| {
            let bytes = verum_vbc::serialize::serialize_module(&source()).expect("wire writer");
            let module = verum_vbc::deserialize::deserialize_module(&bytes).expect("wire reader");
            assert_real_mutable_body(&module);
            assert_runtime_carrier_callers(&module);
            Arc::new(module)
        })
        .clone()
}

fn assert_real_mutable_body(module: &VbcModule) {
    let candidates: List<_> = module
        .functions
        .iter()
        .filter(|function| {
            matches!(
                module.get_string(function.name),
                Some("Map.get_mut" | "core.collections.map.Map.get_mut")
            )
        })
        .collect();
    assert_eq!(candidates.len(), 1, "one real source Map.get_mut body");
    let function = candidates[0];
    assert!(function.has_source_body && function.bytecode_length > 1);
    let start = function.bytecode_offset as usize;
    let end = start + function.bytecode_length as usize;
    let instructions = decode_instructions(&module.bytecode[start..end]).expect("get_mut body");
    assert!(
        instructions
            .iter()
            .any(|instruction| matches!(instruction, Instruction::GetF { field_idx: 2, .. })),
        "source Slot.hash access retained"
    );
    assert!(
        instructions
            .iter()
            .any(|instruction| matches!(instruction, Instruction::LoadI { value: 32, .. })),
        "source Slot stride retained"
    );
}

fn assert_runtime_carrier_callers(module: &VbcModule) {
    for name in [
        "read_control",
        "direct_write_through",
        "empty_and_missing_keys",
        "forwarded_mutable_receiver_and_key",
        "record_field_receiver_and_key",
        "whole_variant_replacement",
        "nested_payload_mutation",
        "collision_chain",
        "shared_tag_scalar",
        "grow_after_borrow_ends",
    ] {
        let qualified = format!("mutable_map_values.{name}");
        let function = module
            .functions
            .iter()
            .find(|function| module.get_string(function.name) == Some(qualified.as_str()))
            .expect("exact caller body");
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let instructions = decode_instructions(&module.bytecode[start..end]).expect("caller body");
        assert!(
            instructions
                .iter()
                .any(|instruction| matches!(instruction, Instruction::NewMap { .. })),
            "{name}: real literal constructor selects runtime Map carrier"
        );
        if name != "read_control" {
            assert!(
                instructions.iter().any(|instruction| match instruction {
                    Instruction::CallM { method_id, .. } => module
                        .get_string(verum_vbc::types::StringId(*method_id))
                        .is_some_and(|method| method == "get_mut" || method.ends_with(".get_mut")),
                    // Two cases intentionally call the parsed forwarding helper.
                    Instruction::Call { func_id, .. } =>
                        module
                            .get_function(verum_vbc::module::FunctionId(*func_id))
                            .and_then(|callee| module.get_string(callee.name))
                            == Some("mutable_map_values.replace"),
                    _ => false,
                }),
                "{name}: selected mutable method or reference-forwarding helper"
            );
        }
    }
}

fn check(name: &str, expected: i64) {
    let mut failures: List<Text> = List::new();
    for (route, module) in [("source", source()), ("wire", wire())] {
        let qualified = format!("mutable_map_values.{name}");
        let entry = module
            .functions
            .iter()
            .find(|function| module.get_string(function.name) == Some(qualified.as_str()))
            .expect("exact caller entry")
            .id;
        match Interpreter::new(module).execute_function(entry) {
            Ok(value) if value.is_int() && value.as_i64() == expected => {}
            outcome => failures
                .push(format!("{route}/{name}: expected {expected}, got {outcome:?}").into()),
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn existing_owned_lookup_control() {
    check("read_control", 719);
}
#[test]
fn keys_and_values_preserve_the_source_iterator_contract() {
    check("key_and_value_iterators", 1830);
}
#[test]
fn source_collection_helpers_consume_key_and_value_iterators() {
    check("collected_keys_and_values", 1830);
}
#[test]
fn iterators_preserve_record_values_and_handle_unallocated_maps() {
    check("record_values_and_fresh_empty_iterators", 40426);
}
#[test]
fn scalar_replacement_writes_the_original_value_slot() {
    check("direct_write_through", 4319);
}
#[test]
fn absent_keys_never_create_a_reference_or_entry() {
    check("empty_and_missing_keys", 7);
}
#[test]
fn forwarded_map_and_key_references_keep_the_original_slot() {
    check("forwarded_mutable_receiver_and_key", 43);
}
#[test]
fn record_field_map_and_key_references_keep_the_original_slot() {
    check("record_field_receiver_and_key", 43);
}
#[test]
fn replacing_a_variant_replaces_the_stored_value() {
    check("whole_variant_replacement", 61);
}
#[test]
fn nested_variant_mutation_is_observable_through_the_original_map() {
    check("nested_payload_mutation", 173711);
}

#[test]
fn colliding_keys_and_missing_chain_keep_their_own_slots() {
    check("collision_chain", 743);
}
#[test]
fn scalar_with_shared_type_bits_is_still_a_value_slot() {
    check("shared_tag_scalar", 731);
}
#[test]
fn growth_after_the_borrow_ends_preserves_prior_writes() {
    check("grow_after_borrow_ends", 211);
}
