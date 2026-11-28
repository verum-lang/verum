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
fn retained_field_preserves_the_allocation_write_grant_in_native_image() {
    native(include_str!("../../verum_vbc/tests/fixtures/retained_allocation.vr"));
}

fn readonly_allocation(operation: &str) {
    let source = include_str!("../../verum_vbc/tests/fixtures/retained_allocation.vr");
    let prefix = source.split("fn main()").next().unwrap();
    native(&format!(r#"{prefix}
            fn main() -> Int {{
                let value = 73;
                let allocation = Allocation {{ strong: 1, neighbor: 11 }};
                let pointer = &allocation as &unsafe mut Allocation;
                let ticket = Ticket {{ ptr: pointer, value: &value }};
                let alias = &ticket;
                let refused = try {{ {operation} 0 }} recover |_| {{ 1 }};
                if refused != 1 || allocation.strong != 1 || allocation.neighbor != 11 {{ return 0; }}
                *alias.value
            }}
        "#));
}

#[test]
fn readonly_allocation_rejects_ordinary_write_in_native_image() {
    readonly_allocation("unsafe { (*pointer).strong = 8; }");
}

#[test]
fn a_local_readonly_field_cannot_supply_a_retained_counter() {
    readonly_allocation(
        r#"let retained = unsafe { @intrinsic("retain_copy", alias, &mut (*pointer).strong) };"#,
    );
}

#[test]
fn a_declared_raw_scalar_store_preserves_the_borrow_grant() {
    let wire = module(r#"
        fn main() -> Int {
            let value = 73;
            let pointer = &value as &unsafe mut Int;
            let refused = try { unsafe { *pointer = 11; } false } recover |_| { true };
            if refused { value } else { 0 }
        }
    "#);
    let body = wire.get_function(wire.find_function_by_name("main").unwrap()).unwrap();
    let start = body.bytecode_offset as usize;
    let end = start + body.bytecode_length as usize;
    let instructions = verum_vbc::bytecode::decode_instructions(&wire.bytecode[start..end]).unwrap();
    assert!(instructions.iter().any(|instruction| matches!(instruction,
        verum_vbc::Instruction::MemExtended { sub_op, .. }
            if *sub_op == verum_vbc::instruction::MemSubOpcode::DerefMutRaw as u8)));
    source_image::run_module(wire, |_| {});
}

#[test]
fn a_reassigned_raw_reference_uses_the_selected_borrow() {
    native(r#"
        fn probe(readonly: Bool) -> Bool {
            let original = 7;
            let mut target = 11;
            let mut pointer = &original as &unsafe mut Int;
            if !readonly { pointer = &mut target as &unsafe mut Int; }
            let observed = unsafe { *pointer };
            let changed = try { unsafe { *pointer = 31; } true } recover |_| { false };
            if original != 7 { return false; }
            if readonly { observed == 7 && !changed && target == 11 }
            else { observed == 11 && changed && target == 31 }
        }
        fn main() -> Int { if probe(true) && probe(false) { 73 } else { 0 } }
    "#);
}

#[test]
fn an_uncaught_local_readonly_write_reports_the_missing_grant() {
    let wire = module(r#"
        type Counter is { value: Int };
        fn main() -> Int {
            let counter = Counter { value: 7 };
            let pointer = &counter as &unsafe mut Counter;
            unsafe { (*pointer).value = 31; }
            counter.value
        }
    "#);
    let child = source_image::execute_module(wire, |_| {});
    assert!(!child.status.success());
    let reason = verum_common::Text::from_utf8_lossy(&child.stderr);
    assert!(reason.contains("CBGR: write through a read-only reference"), "{reason}");
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
