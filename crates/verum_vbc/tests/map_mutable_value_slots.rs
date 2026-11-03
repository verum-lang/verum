//! T1196: mutable Map access must alias the value slot owned by its runtime carrier.
//!
//! Compile the complete production Map module and its caller without filtering or
//! replacing the Slot/get_mut declarations. List and primitive bodies are also
//! compiled so collection helpers execute the real Drop and Clone implementations. The mount
//! loader supplies other dependency declarations. Callers use real map literals (NewMap) to select the runtime
//! carrier; source-only Map.new takes a different route from the ordinary
//! archive compiler. The adversarial bytecode test replaces only its own caller helper. Each executed
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
const PRODUCTION_LIST: &str = include_str!("../../../core/collections/list.vr");
const PRODUCTION_PRIMITIVES: &str = include_str!("../../../core/base/primitives.vr");
const CALLER: &str = r#"
module mutable_map_values;
mount core.collections.map.Map;
mount core.collections.set.Set;
mount core.mem.allocator.AllocError;

type Cell is { padding: Int, value: Int };
type Options is { version: Int, features: Maybe<Cell>, optional: Maybe<Bool> };
type Dependency is Requirement(Int) | Detailed(Options);
type Holder is { padding: Int, values: Map<Text, Int>, key: Text };

fn unit_keys_are_values() -> Int {
    let mut values: Map<Unit, Int> = { (): 17 };
    let before = match values.get(()) { Maybe.Some(n) => n, Maybe.None => -7 };
    let old = match values.insert((), 25) { Maybe.Some(n) => n, Maybe.None => -13 };
    assert_eq(values.len(), 1);
    assert(matches_unit_entry(&values));
    assert(values.try_reserve(64) is Result.Ok(_));
    let removed = match values.remove(()) { Maybe.Some(n) => n, Maybe.None => -19 };
    assert_eq(values.len(), 0);
    before * 1000 + old * 100 + removed
}

fn matches_unit_entry(values: &Map<Unit, Int>) -> Bool {
    match values.get_key_value(&()) {
        Maybe.Some(pair) => *pair.0 == () && *pair.1 == 25,
        Maybe.None => false,
    }
}

fn unit_set_membership() -> Int {
    let mut values: Set<Unit> = Set.new();
    assert(values.insert(()));
    assert(!values.insert(()));
    assert(values.contains(()));
    assert_eq(values.len(), 1);
    assert(values.remove(()));
    assert(!values.contains(()));
    assert(values.insert(()));
    values.clear();
    values.len()
}

fn read_number(values: &Map<Int, Int>, key: Int) -> Int {
    match values.get(key) { Maybe.Some(n) => n, Maybe.None => panic("key missing") }
}

fn fallible_insert_contract() -> Int {
    let mut values: Map<Int,Int> = { 11: 17 };
    let old = match values.try_insert(11, 25) { Result.Ok(Maybe.Some(n)) => n, _ => 0 };
    let fresh = match values.try_insert(12, 44) { Result.Ok(Maybe.None) => 1, _ => 0 };
    assert_eq(read_number(&values, 11), 25);
    old * 1000 + fresh * 100 + read_number(&values, 12)
}

fn conditional_insert_contract() -> Int {
    let mut values: Map<Int, Int> = { 1: 17, 17: 36 };
    let rejected = match values.insert_if_absent(1, 25) { Result.Err(n) => n, _ => 0 };
    // All three keys start in the same bucket of the literal's table.
    match values.insert_if_absent(33, 44) {
        Result.Ok(slot) => { *slot = 51; },
        Result.Err(_) => panic("absent key rejected"),
    }
    assert_eq(read_number(&values, 17), 36);
    rejected * 1000 + read_number(&values, 1) * 100 + read_number(&values, 33)
}

fn fallible_growth_preserves_entries() -> Int {
    let mut values: Map<Int, Int> = { 3: 17 };
    let old_capacity = values.capacity();
    let requested = match values.try_reserve(100000000) {
        Result.Err(AllocError.OutOfMemory { requested }) => requested,
        _ => 0,
    };
    assert(requested >= 1600000000);
    assert_eq(values.capacity(), old_capacity);
    assert_eq(values.len(), 1);
    read_number(&values, 3)
}

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

fn inspect_slot(value: &Int) -> Int { *value }
fn pass_slot() -> Int {
    let values: Map<Int, Int> = { 7: 41 };
    match values.get_key_value(&7) {
        Maybe.Some(pair) => inspect_slot(pair.1),
        Maybe.None => 0,
    }
}

fn borrowed_entry() -> Int {
    let values: Map<Int, Int> = { 1: 7, 17: 19 };
    assert(values.get_key_value(&33) is Maybe.None);
    match values.get_key_value(&17) {
        Maybe.Some(entry) => *entry.0 * 100 + *entry.1,
        Maybe.None => -1,
    }
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


type DropCounter is { count: &mut Int };
implement Drop for DropCounter {
    fn drop(&mut self) { *self.count = *self.count + 1; }
}
type NestedCounter is { inner: DropCounter };
type OwnedKey is { id: Int, guard: DropCounter };
implement Eq for OwnedKey {
    fn eq(&self, other: &OwnedKey) -> Bool { self.id == other.id }
}
implement Hash for OwnedKey {
    fn hash(&self, hasher: &mut Hasher) { hasher.write_int(self.id); }
}

fn destroy_owned_entries(count: &mut Int) {
    let values: Map<Int, NestedCounter> = {
        1: NestedCounter { inner: DropCounter { count: count } },
        17: NestedCounter { inner: DropCounter { count: count } },
    };
}

fn destroy_borrowed_entry(value: &DropCounter) {
    let values: Map<Int, &DropCounter> = { 1: value };
}

fn borrowed_entry_survives() -> Int {
    let mut count = 0;
    let guard = DropCounter { count: &mut count };
    destroy_borrowed_entry(&guard);
    count
}

fn destroy_owned_key(count: &mut Int) {
    let key = OwnedKey { id: 1, guard: DropCounter { count: count } };
    let values: Map<OwnedKey, Int> = { (key): 19 };
}

fn owned_key_dropped_once() -> Int {
    let mut count = 0;
    destroy_owned_key(&mut count);
    count
}


fn escape_local_entry() -> Maybe<(&Int, &Int)> {
    let values: Map<Int, Int> = { 7: 41 };
    values.get_key_value(&7)
}

fn read_after_drop() -> Int {
    match escape_local_entry() { Maybe.Some(pair) => *pair.1, Maybe.None => 0 }
}

fn read_after_remove_reinsert() -> Int {
    let mut values: Map<Int, Int> = { 7: 41 };
    match values.get_key_value(&7) {
        Maybe.Some(pair) => {
            values.remove(7);
            values.insert(7, 99);
            *pair.1
        },
        Maybe.None => 0,
    }
}

fn read_after_growth() -> Int {
    let mut values: Map<Int, Int> = { 7: 41 };
    match values.get_key_value(&7) {
        Maybe.Some(pair) => {
            values.reserve(2048);
            *pair.0
        },
        Maybe.None => 0,
    }
}

fn write_after_clear() -> Int {
    let mut values: Map<Int, Int> = { 7: 41 };
    match values.get_mut(&7) {
        Maybe.Some(value) => {
            values.clear();
            *value = 42;
            42
        },
        Maybe.None => 0,
    }
}

fn owned_entries_dropped_once() -> Int {
    let mut count = 0;
    destroy_owned_entries(&mut count);
    count
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
            let list = Parser::new(PRODUCTION_LIST)
                .parse_module()
                .expect("production List implementation");
            // Mounts installed the declarations. The exercised collection helpers
            // also need the executable Int.clone and List.drop bodies.
            codegen
                .compile_unit_items(&[&primitives, &list], ItemFailurePolicy::Strict)
                .expect("complete primitive and List bodies");
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
fn unit_is_a_valid_hash_key_across_lookup_growth_and_removal() {
    check("unit_keys_are_values", 18725);
}

#[test]
fn unit_is_a_valid_set_element_across_insert_remove_and_clear() {
    check("unit_set_membership", 0);
}

#[test]
fn fallible_insert_returns_the_replaced_value() {
    check("fallible_insert_contract", 17144);
}
#[test]
fn vacant_insert_borrows_its_own_collision_slot() {
    check("conditional_insert_contract", 26751);
}
#[test]
fn rejected_growth_returns_typed_allocation_failure_and_preserves_map() {
    check("fallible_growth_preserves_entries", 17);
}
#[test]
fn shared_entry_lookup_reads_both_stored_slots() {
    check("borrowed_entry", 1719);
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


#[test]
fn flat_map_runs_nested_value_destructors_once() {
    check("owned_entries_dropped_once", 2);
}

#[test]
fn flat_map_runs_owned_key_destructor_once() {
    check("owned_key_dropped_once", 1);
}

#[test]
fn flat_map_leaves_borrowed_values_alive() {
    check("borrowed_entry_survives", 0);
}

#[test]
fn managed_map_slots_reject_reads_and_writes_after_ownership_ends() {
    use verum_vbc::interpreter::{CbgrViolationKind, InterpreterError};
    for (route, module) in [("source", source()), ("wire", wire())] {
        for name in [
            "read_after_drop",
            "read_after_remove_reinsert",
            "read_after_growth",
            "write_after_clear",
        ] {
            let entry = module
                .find_function_by_name(&format!("mutable_map_values.{name}"))
                .unwrap();
            let error = Interpreter::new(module.clone())
                .execute_function(entry)
                .unwrap_err();
            assert!(
                matches!(
                    error,
                    InterpreterError::CbgrViolation {
                        kind: CbgrViolationKind::UseAfterFree,
                        ..
                    }
                ),
                "{route}/{name}: {error}"
            );
        }
    }
}

#[test]
fn raw_bytecode_cannot_export_a_live_managed_slot() {
    use verum_vbc::bytecode::encode_instruction;
    use verum_vbc::instruction::CbgrSubOpcode;
    use verum_vbc::interpreter::InterpreterError;

    let mut module = (*source()).clone();
    let caller = module
        .find_function_by_name("mutable_map_values.pass_slot")
        .unwrap();
    let callee = module
        .find_function_by_name("mutable_map_values.inspect_slot")
        .unwrap();
    let start = module.bytecode.len() as u32;
    encode_instruction(
        &Instruction::CbgrExtended {
            sub_op: CbgrSubOpcode::RefRawAddr as u8,
            operands: vec![1, 0],
        },
        &mut module.bytecode,
    );
    encode_instruction(&Instruction::RetV, &mut module.bytecode);
    let body = &mut module.functions[callee.0 as usize];
    body.bytecode_offset = start;
    body.bytecode_length = module.bytecode.len() as u32 - start;
    body.register_count = body.register_count.max(2);
    body.instructions = None;
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    for module in [module, wire] {
        let error = Interpreter::new(Arc::new(module))
            .execute_function(caller)
            .unwrap_err();
        assert!(
            matches!(error, InterpreterError::InvalidOperand { ref message } if message.contains("managed slot")),
            "{error}"
        );
    }
}
