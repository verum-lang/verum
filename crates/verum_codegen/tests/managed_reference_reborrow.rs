//! A managed reborrow preserves the selected storage across native call frames.
#![cfg(target_os = "macos")]
#[path = "support/source_image.rs"]
mod source_image;

#[test]
fn scalar_reborrow_updates_the_original_field_after_return() {
    source_image::run(r#"
        type Pair is { padding: Int, value: Int };
        fn reborrow(value: &mut Int) -> &mut Int { &mut *value }
        fn main() -> Int {
            let mut pair = Pair { padding: 11, value: 7 };
            let target = reborrow(&mut pair.value);
            *target = 73;
            if pair.padding != 11 { return 0; }
            pair.value
        }
    "#);
}

#[test]
fn record_reborrow_preserves_nested_storage_across_calls() {
    source_image::run(r#"
        type Inner is { value: Int }; type Outer is { inner: Inner };
        fn reborrow(value: &mut Inner) -> &mut Inner { &mut *value }
        fn field(value: &mut Inner) -> &mut Int { &mut value.value }
        fn main() -> Int {
            let mut outer = Outer { inner: Inner { value: 7 } };
            let target = field(reborrow(&mut outer.inner));
            *target = 73;
            outer.inner.value
        }
    "#);
}

#[test]
fn shared_reborrow_keeps_the_original_value() {
    source_image::run(r#"
        fn reborrow(value: &Int) -> &Int { &*value }
        fn main() -> Int { let value = 73; *reborrow(&value) }
    "#);
}

#[test]
fn shared_indexed_records_survive_a_sibling_borrow_at_a_call() {
    source_image::run(r#"
        type Item is { amount: Int };
        fn sum(left: &Item, right: &Item) -> Int { left.amount + right.amount }
        fn main() -> Int {
            let items: List<Item> = [Item { amount: 8 }, Item { amount: 11 }];
            if sum(&items[0], &items[1]) == 19 { 73 } else { 0 }
        }
    "#);
}

#[test]
fn raw_reference_to_a_mutable_local_supports_field_access_and_whole_value_replacement() {
    let wire = source_image::module(r#"
        type Pair is { first: Int, second: Int };
        fn main() -> Int {
            let mut pair = Pair { first: 3, second: 5 };
            let pointer = &mut pair as &unsafe mut Pair;
            unsafe {
                if (*pointer).first != 3 || (*pointer).second != 5 { return 0; }
                (*pointer).first = 42;
            }
            if pair.first != 42 || pair.second != 5 { return 0; }
            unsafe { *pointer = Pair { first: 42, second: 31 }; }
            let shared = &pair as &unsafe Pair;
            unsafe { (*shared).first + (*shared).second }
        }
    "#);
    let entry = wire.find_function_by_name("main").unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::new(wire.clone().into());
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    source_image::run_module(wire, |_| {});
}
