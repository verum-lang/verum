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
