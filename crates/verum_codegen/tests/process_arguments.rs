//! The native startup table and VBC argument operations share an indexing contract.
#![cfg(target_os = "macos")]
#[path = "support/source_image.rs"]
mod source_image;

#[test]
fn linked_argv_is_available_without_claiming_user_function_names() {
    source_image::run(
        r#"
        module program;
        @intrinsic("process_arg_count") fn count() -> Int;
        @intrinsic("process_arg_bytes") fn read(index: Int) -> &[Byte];
        fn args() -> Int { 40 }
        fn args_count() -> Int { 12 }
        fn arg(index: Int) -> Int { index + 2 }
        fn __args_count_raw() -> Int { 5 }
        fn __arg_raw(n: Int) -> Int { 2 * n }
        fn main() -> Int {
            if count() != 1 || read(0).len() == 0 { return 1; }
            args() + args_count() + arg(4) + __args_count_raw() + __arg_raw(5)
        }
    "#,
    );
}

#[test]
fn linked_argv_preserves_empty_unicode_and_non_unicode_byte_sequences() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let module = source_image::module(
        r#"
        module program;
        @intrinsic("process_arg_count") fn count() -> Int;
        @intrinsic("process_arg_bytes") fn read(index: Int) -> &[Byte];
        fn main() -> Int {
            if count() != 5 || read(1).len() != 0 || read(2).len() != 2 { return 1; }
            let unicode = read(3);
            let raw = read(4);
            if read(2)[0] != 0x2D as Byte || unicode.len() != 2 || unicode[1] != 0xB4 as Byte { return 2; }
            if raw.len() != 4 || raw[1] != 0xFF as Byte || raw[3] != 0x9F as Byte { return 3; }
            73
        }
    "#,
    );
    let arguments = [
        OsStr::new(""),
        OsStr::new("--"),
        OsStr::new("δ"),
        OsStr::from_bytes(&[97, 255, 240, 159]),
    ];
    let output = source_image::execute_with_arguments(module, |_| {}, &arguments);
    assert_eq!(
        output.status.code(),
        Some(73),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn linked_program_reads_its_own_environment() {
    let module = source_image::module(include_str!(
        "../../verum_vbc/tests/fixtures/process_environment.vr"
    ));
    let output = source_image::execute_with_command(module, |_| {}, |command| {
        command.env_clear().env("A", "").env("B", "x=y");
    });
    assert_eq!(
        output.status.code(),
        Some(73),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
