//! A declared retained wrapper preserves its source on success and refusal.
#![cfg(target_os = "macos")]
#[path = "support/source_image.rs"]
mod source_image;
use source_image::{module, run as native};

const SOURCE: &str = r#"
    type Handle is { generation: UInt32, value: &Int };
    type Counter is { strong: UInt32 };
    fn retain(handle: &Handle, count: &mut UInt32) -> Handle {
        unsafe { @intrinsic("retain_copy", handle, count) }
    }
    fn main() -> Int {
        let value = 73;
        let mut handle = Handle { generation: 2, value: &value };
        let mut counts = Counter { strong: 1 };
        let copied = retain(&handle, &mut counts.strong);
        if counts.strong != 2 { return 0; }
        handle.generation = 9;
        if copied.generation != 2 { return 0; }
        *copied.value
    }
"#;

#[test]
fn retain_copy_preserves_the_borrowed_handle_in_vm() {
    let wire = module(SOURCE);
    let entry = wire.find_function_by_name("main").unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::new(wire.into());
    let value = vm.execute_function(entry).expect("VM execution");
    assert_eq!(value.try_as_i64(), Some(73));
}

#[test]
fn retained_handle_runs_in_native_image() {
    native(SOURCE);
}

#[test]
fn caught_zero_and_overflow_preserve_the_counter_in_native_image() {
    for count in [0, u32::MAX] {
        let prefix = SOURCE.split("    fn main()").next().unwrap();
        native(&format!(
            r#"{prefix}
            fn main() -> Int {{
                let value = 73;
                let handle = Handle {{ generation: 2, value: &value }};
                let mut counts = Counter {{ strong: {count} }};
                let refused = try {{
                    let copied = retain(&handle, &mut counts.strong);
                    0
                }} recover |_| {{ 1 }};
                if refused != 1 {{ return 0; }}
                if counts.strong != {count} {{ return 1; }}
                *handle.value
            }}
        "#
        ));
    }
}

#[test]
fn empty_record_retains_without_inventing_a_native_field() {
    native(
        r#"type Handle is {}; fn main()->Int {
        let handle=Handle{}; let mut count:UInt32=1;
        let copied=unsafe { @intrinsic("retain_copy", &handle, &mut count) };
        if count==2 {73} else {0}
    }"#,
    );
}
