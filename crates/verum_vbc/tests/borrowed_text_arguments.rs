//! T1757: borrowed Text fields must supply their contents to Text operations.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::VbcCodegen;
use verum_vbc::interpreter::Interpreter;

const HELPERS: &str = r#"
type Fields is { padding: Int, word: Text };
fn snapshot_text(source: &Text) -> Text {
    let mut result = "";
    result.push_str(source);
    result
}
fn borrowed_field(source: &Fields) -> Text { snapshot_text(&source.word) }
fn forwarded(source: &Text) -> Text { snapshot_text(source) }
"#;

fn check(body: &str, expected: &str) {
    let source = format!("{HELPERS}\nfn probe() -> Text {{ {body} }}");
    let ast = Parser::new(&source)
        .parse_module()
        .expect("fixture grammar");
    let module = VbcCodegen::new()
        .compile_module(&ast)
        .expect("source codegen");
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize"),
    )
    .expect("decode wire");
    let mut failures = Vec::new();
    for (route, module) in [("source", module), ("wire", wire)] {
        let entry = module.find_function_by_name("probe").expect("probe");
        let mut interpreter = Interpreter::new(Arc::new(module));
        let actual = interpreter.execute_function(entry).expect("execute");
        let actual = interpreter.read_text(actual);
        if actual.as_deref() != Some(expected) {
            failures.push(format!("{route}: expected {expected:?}, got {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn local_reference_preserves_text() {
    check(r#"let word = "widget"; snapshot_text(&word)"#, "widget");
}

#[test]
fn owned_record_field_supplies_text_contents() {
    check(
        r#"let fields = Fields { padding: 17, word: "widget" }; snapshot_text(&fields.word)"#,
        "widget",
    );
}

#[test]
fn borrowed_record_field_supplies_text_contents() {
    check(
        r#"let fields = Fields { padding: 17, word: "widget" }; borrowed_field(&fields)"#,
        "widget",
    );
}

#[test]
fn forwarded_record_field_supplies_text_contents() {
    check(
        r#"let fields = Fields { padding: 17, word: "widget" }; forwarded(&fields.word)"#,
        "widget",
    );
}

#[test]
fn heap_text_field_preserves_unicode_bytes() {
    check(
        r#"let fields = Fields { padding: 17, word: "registry λ — publication" }; borrowed_field(&fields)"#,
        "registry λ — publication",
    );
}

#[test]
fn optional_payload_preserves_text() {
    check(
        r#"
        let word = Maybe.Some("λ");
        match &word {
            Maybe.Some(text) => snapshot_text(text),
            Maybe.None => "missing",
        }
    "#,
        "λ",
    );
}

#[test]
fn list_element_preserves_text() {
    check(
        r#"
        let mut words: List<Text> = List.new();
        words.push("Alice");
        for word in words.iter() { return snapshot_text(word); }
        "missing"
    "#,
        "Alice",
    );
}

#[test]
fn empty_field_does_not_render_its_address() {
    check(
        r#"let fields = Fields { padding: 17, word: "" }; borrowed_field(&fields)"#,
        "",
    );
}

#[test]
fn replacement_uses_borrowed_field_contents() {
    check(
        r#"let fields = Fields { padding: 17, word: "widget" }; "widget widget".replace(&fields.word, &"cog")"#,
        "cog cog",
    );
}
#[test]
fn reserve_from_tracked_field_preserves_partial_utf8_bytes() {
    use std::sync::Arc;
    use verum_common::List;
    use verum_vbc::bytecode::encode_instruction;
    use verum_vbc::instruction::{CbgrSubOpcode, Instruction, Reg, RegRange};
    use verum_vbc::interpreter::{
        Interpreter, OBJECT_HEADER_SIZE, ObjectHeader, TEXT_RECORD_SIZE, text_record_cap,
        text_record_payload,
    };
    use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};
    use verum_vbc::types::{FieldDescriptor, TypeDescriptor, TypeId, TypeKind, TypeRef};
    use verum_vbc::value::Value;

    const HOLDER: TypeId = TypeId(36_161);
    const PARTIAL: &[u8] = &[b'a', 0xe2, 0x82];
    const ADDITIONAL: i8 = 8;

    let mut module = VbcModule::new("reserve_field_bytes".into());
    let mut holder = TypeDescriptor {
        id: HOLDER,
        name: module.intern_string("Holder"),
        kind: TypeKind::Record,
        size: 4 * std::mem::size_of::<Value>() as u32,
        alignment: std::mem::align_of::<Value>() as u32,
        ..Default::default()
    };
    for (index, name) in ["padding", "word", "tail_a", "tail_b"]
        .into_iter()
        .enumerate()
    {
        holder.fields.push(FieldDescriptor {
            name: module.intern_string(name),
            type_ref: TypeRef::Concrete(if index == 1 {
                TypeId::TEXT
            } else {
                TypeId::INT
            }),
            offset: (index * std::mem::size_of::<Value>()) as u32,
            ..Default::default()
        });
    }
    module.types.push(holder);
    for instruction in [
        // r0 is a live Holder. Produce the interior pointer through dispatch;
        // never insert a fabricated address into cbgr_mutable_ptrs.
        Instruction::CbgrExtended {
            sub_op: CbgrSubOpcode::RefField as u8,
            operands: List::from(&[1_u8, 0, 1][..]).into(),
        },
        Instruction::RefMut {
            dst: Reg(2),
            src: Reg(1),
        },
        Instruction::LoadSmallI {
            dst: Reg(3),
            value: ADDITIONAL,
        },
        Instruction::Call {
            dst: Reg(4),
            func_id: 1,
            args: RegRange {
                start: Reg(2),
                count: 2,
            },
        },
        // Read the same retained field reference after reserve. Both byte
        // preservation and writeback must hold through actual dispatch.
        Instruction::Deref {
            dst: Reg(5),
            ref_reg: Reg(1),
        },
        Instruction::Ret { value: Reg(5) },
    ] {
        encode_instruction(&instruction, &mut module.bytecode);
    }
    let mut probe = FunctionDescriptor::new(module.intern_string("probe"));
    probe.id = FunctionId(0);
    probe.bytecode_length = module.bytecode.len() as u32;
    probe.register_count = 6;
    module.functions.push(probe);
    let mut reserve = FunctionDescriptor::new(module.intern_string("Text.reserve"));
    reserve.id = FunctionId(1);
    reserve.bytecode_offset = module.bytecode.len() as u32;
    reserve.bytecode_length = 1;
    reserve.register_count = 2;
    encode_instruction(&Instruction::RetV, &mut module.bytecode);
    module.functions.push(reserve);

    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize fixture"),
    )
    .expect("decode fixture");
    let mut failures = Vec::new();
    for (route, module) in [("direct bytecode", module), ("decoded wire", wire)] {
        let mut interpreter = Interpreter::new(Arc::new(module));
        let text = interpreter
            .state
            .heap
            .alloc_text(PARTIAL)
            .expect("heap Text");
        let original = Value::from_ptr(text.as_ptr().cast::<u8>());
        let holder = interpreter
            .state
            .heap
            .alloc(HOLDER, 4 * std::mem::size_of::<Value>())
            .expect("live Holder");
        let slots = holder.data_ptr().cast::<Value>();
        // SAFETY: all four slots belong to the live Holder allocation.
        unsafe {
            for index in 0..4 {
                *slots.add(index) = Value::from_i64(0);
            }
            *slots.add(1) = original;
        }
        // SAFETY: field 1 is inside the four-slot Holder allocation.
        let field_address = unsafe { slots.add(1) } as usize;
        assert!(!interpreter.state.cbgr_mutable_ptrs.contains(&field_address));
        let result = interpreter
            .execute_function_with_args(
                FunctionId(0),
                &[Value::from_ptr(holder.as_ptr().cast::<u8>())],
            )
            .expect("dispatch RefField, RefMut and static Text.reserve");
        assert!(interpreter.state.cbgr_mutable_ptrs.contains(&field_address));
        // SAFETY: the Holder allocation remains live during this execution.
        assert_eq!(
            unsafe { *slots.add(1) }.bits(),
            result.bits(),
            "{route}: replacement reached source slot"
        );

        assert_ne!(
            result.bits(),
            original.bits(),
            "{route}: reserve replacement"
        );
        assert!(result.is_ptr(), "{route}: capacity record");
        let base = result.as_ptr::<u8>();
        assert!(
            interpreter.state.heap.contains(base.cast::<ObjectHeader>()),
            "{route}: owned result"
        );
        let object = interpreter
            .state
            .heap
            .get_object(base.wrapping_add(OBJECT_HEADER_SIZE))
            .expect("exact live result allocation");
        assert_eq!(object.type_id(), TypeId::TEXT, "{route}: Text producer");
        assert!(object.size() as usize >= TEXT_RECORD_SIZE);
        // SAFETY: exact heap ownership, TEXT identity and record size checked.
        let (bytes, len) = unsafe { text_record_payload(base) }.expect("Text payload");
        assert_eq!(
            bytes,
            object.data_ptr().wrapping_add(TEXT_RECORD_SIZE),
            "{route}: inline bytes"
        );
        assert!(
            len <= object.data_slice().len() - TEXT_RECORD_SIZE,
            "{route}: bounded bytes"
        );
        // SAFETY: the same live canonical Text record established above.
        assert!(unsafe { text_record_cap(base) } >= (PARTIAL.len() + ADDITIONAL as usize) as i64);
        let actual = &object.data_slice()[TEXT_RECORD_SIZE..TEXT_RECORD_SIZE + len];
        if actual != PARTIAL {
            failures.push(format!("{route}: expected {PARTIAL:?}, got {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
