//! T1812: a mutating field call writes to the same projected owner it reads.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

const TYPES: &str = r#"
type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
type Slot is { value: Int };
implement Slot {
    fn take(&mut self) -> Int { let previous = self.value; *self = Slot { value: 0 }; previous }
}
type Store is { left: Slot, right: Slot };
type Guard<T> is { target: &mut T, noise: Int };
implement<T> Deref for Guard<T> {
    type Target = T;
    fn deref(&self) -> &T { self.target }
}
"#;

fn run(source: &str) -> i64 {
    let source = format!("{TYPES}\n{source}");
    let ast = Parser::new(&source).parse_module().unwrap();
    let mut values = [0; 2];
    for (index, bootstrap) in [false, true].into_iter().enumerate() {
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("field_writeback"));
        let module = if bootstrap {
            cg.collect_unit_declarations(&[&ast]).unwrap();
            cg.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
            cg.finalize_module_from_state().unwrap()
        } else { cg.compile_module(&ast).unwrap() };
        let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
        let entry = module.find_function_by_name("field_writeback.probe").unwrap();
        values[index] = Interpreter::new(Arc::new(module)).execute_function(entry).unwrap().as_i64();
    }
    assert_eq!(values[0], values[1], "both source producers must agree");
    values[0]
}

#[test]
fn deref_field_replacement_preserves_the_guard_and_updates_the_pointee() {
    assert_eq!(run(r#"
fn probe() -> Int {
    let mut store = Store { left: Slot { value: 17 }, right: Slot { value: 23 } };
    let mut guard: Guard<Store> = Guard { target: &mut store, noise: 91 };
    let left = guard.left.take();
    let right = guard.right.take();
    assert_eq(guard.noise + 0, 91);
    assert_eq(store.left.value + 0, 0);
    assert_eq(store.right.value + 0, 0);
    left + right
}
"#), 40);
}

#[test]
fn field_method_evaluates_its_base_once() {
    assert_eq!(run(r#"
type Count is { value: Int };
fn select(count: &mut Count, store: &mut Store) -> &mut Store {
    count.value = count.value + 1;
    store
}
fn probe() -> Int {
    let mut count = Count { value: 0 };
    let mut store = Store { left: Slot { value: 17 }, right: Slot { value: 23 } };
    let value = select(&mut count, &mut store).left.take();
    assert_eq(store.left.value + 0, 0);
    count.value * 100 + value
}
"#), 117);
}

#[test]
fn a_wrappers_own_field_precedes_its_deref_target() {
    assert_eq!(run(r#"
type OwnGuard is { left: Slot, target: &mut Store };
implement Deref for OwnGuard {
    type Target = Store;
    fn deref(&self) -> &Store { self.target }
}
fn probe() -> Int {
    let mut store = Store { left: Slot { value: 17 }, right: Slot { value: 23 } };
    let mut guard = OwnGuard { left: Slot { value: 31 }, target: &mut store };
    let own = guard.left.take();
    let projected = guard.right.take();
    assert_eq(guard.left.value + 0, 0);
    assert_eq(store.left.value + 0, 17);
    assert_eq(store.right.value + 0, 0);
    own + projected
}
"#), 54);
}
