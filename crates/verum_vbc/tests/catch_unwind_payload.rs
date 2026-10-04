//! T1536: catch errors use the callee's nominal descriptor in both tiers.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::codegen::VbcCodegen;
use verum_vbc::interpreter::{self as heap, Interpreter};
use verum_vbc::module::VbcModule;
use verum_vbc::value::Value;

fn source(owner: &str, error: &str, invocation: &str) -> String {
    format!(
        r#"
module {owner};
type Location is {{ file: Text, line: Int, column: Int }};
type {error} is {{ message: Text, location: Maybe<Location> }};
@intrinsic("catch_unwind")
fn fence<T>(f: fn() -> T) -> Result<T, {error}> {{ @intrinsic("catch_unwind", f) }}
fn victim() -> Int {{ panic("payload-seven"); 0 }}
fn probe() -> Result<Int, {error}> {{ {invocation} }}
"#
    )
}

fn compile(source: &str) -> Result<VbcModule, String> {
    let ast = Parser::new(source).parse_module().expect("valid source");
    VbcCodegen::new()
        .compile_module(&ast)
        .map_err(|e| format!("{e:?}"))
}

fn run(module: VbcModule) -> (Interpreter, Value) {
    let id = module
        .functions
        .iter()
        .find(|fd| {
            module
                .get_string(fd.name)
                .is_some_and(|name| name == "probe" || name.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    let mut interpreter = Interpreter::new(Arc::new(module));
    let result = interpreter
        .execute_function(id)
        .expect("catch must return normally");
    assert_eq!(interpreter.state.call_stack.depth(), 0);
    assert!(interpreter.state.exception_handlers.is_empty());
    (interpreter, result)
}

fn caught_error(value: Value) -> Value {
    assert!(value.is_ptr() && !value.is_nil());
    // SAFETY: a compiled catch returns a freshly allocated Result variant.
    unsafe {
        assert_eq!(heap::variant_tag(value.as_ptr()), 1);
        *(value.as_ptr::<u8>().add(heap::OBJECT_HEADER_SIZE + 8) as *const Value)
    }
}

#[test]
fn both_declared_public_error_shapes_keep_nominal_identity_message_and_none() {
    for (owner, error) in [
        ("core.base.panic", "PanicInfo"),
        ("core.intrinsics.control", "IntrinsicPanicInfo"),
    ] {
        let module = compile(&source(owner, error, "fence(|| victim())")).expect("compile");
        let type_id = module
            .types
            .iter()
            .find(|td| module.get_string(td.name) == Some(error))
            .expect("error type")
            .id;
        let (_interpreter, result) = run(module);
        let error = caught_error(result);
        // SAFETY: the result payload is the two-field catch error record.
        unsafe {
            assert_eq!(
                heap::ObjectHeader::ref_or_stub(error.as_ptr()).type_id,
                type_id
            );
            let fields = error.as_ptr::<u8>().add(heap::OBJECT_HEADER_SIZE) as *const Value;
            let message = *fields;
            let location = *fields.add(1);
            assert_eq!(heap::variant_tag(location.as_ptr()), 0);
            // The interpreter's text heap stores UTF-8 behind its text header.
            let (bytes, len) =
                heap::text_record_payload(message.as_ptr::<u8>()).expect("Text payload");
            assert_eq!(std::slice::from_raw_parts(bytes, len), b"payload-seven");
        }
    }
}

#[test]
fn raw_intrinsic_uses_canonical_origin_not_enclosing_return_type() {
    let text = source(
        "core.intrinsics.control",
        "IntrinsicPanicInfo",
        "@intrinsic(\"catch_unwind\", || victim())",
    ) + "\nfn integer_caller() -> Int { let result = @intrinsic(\"catch_unwind\", || 7); 7 }\n";
    let module = compile(&text).expect("canonical raw intrinsic in an Int caller");
    let (_interpreter, result) = run(module);
    let _ = caught_error(result);
}

#[test]
fn raw_intrinsic_rejects_same_leaf_from_foreign_module() {
    let text = source(
        "foreign.control",
        "IntrinsicPanicInfo",
        "@intrinsic(\"catch_unwind\", || victim())",
    );
    let error = compile(&text).expect_err("a foreign leaf is not canonical authority");
    assert!(
        error.contains("core.intrinsics.control.IntrinsicPanicInfo"),
        "{error}"
    );
}

#[test]
fn invalid_declared_payload_shape_is_rejected() {
    let text = source("core.base.panic", "PanicInfo", "fence(|| victim())")
        .replace("message: Text", "message: Int");
    let error = compile(&text).expect_err("message must be Text");
    assert!(error.contains("error descriptor must declare"), "{error}");
}

#[test]
fn inline_catch_passes_success_and_survives_nested_panics() {
    let text = source(
        "core.base.panic",
        "PanicInfo",
        "fence(|| { let inner = fence(|| victim()); 37 })",
    );
    let (_interpreter, result) = run(compile(&text).expect("compile"));
    // SAFETY: Result.Ok contains one integer payload.
    unsafe {
        assert_eq!(heap::variant_tag(result.as_ptr()), 0);
        assert_eq!(
            (*(result.as_ptr::<u8>().add(heap::OBJECT_HEADER_SIZE + 8) as *const Value)).as_i64(),
            37
        );
    }
}

#[test]
fn inline_handler_does_not_catch_nonpanic_runtime_faults() {
    let mut module = compile(&source(
        "core.base.panic",
        "PanicInfo",
        "fence(|| victim())",
    ))
    .expect("compile");
    let victim = module
        .functions
        .iter_mut()
        .find(|fd| {
            module
                .strings
                .get(fd.name)
                .is_some_and(|name| name == "victim" || name.ends_with(".victim"))
        })
        .expect("victim");
    // An invalid opcode is an interpreter fault, never a language panic.
    module.bytecode[victim.bytecode_offset as usize] = 0xff;
    let id = module
        .functions
        .iter()
        .find(|fd| {
            module
                .get_string(fd.name)
                .is_some_and(|name| name == "probe" || name.ends_with(".probe"))
        })
        .unwrap()
        .id;
    let error = Interpreter::new(Arc::new(module))
        .execute_function(id)
        .expect_err("fault must propagate");
    assert!(!matches!(
        error,
        verum_vbc::interpreter::InterpreterError::Panic { .. }
    ));
}

#[test]
fn inner_catch_does_not_replace_the_later_outer_panic_message() {
    let text = source(
        "core.base.panic",
        "PanicInfo",
        r#"fence(|| {
        let inner = fence(|| { panic("inner message"); 0 });
        panic("outer message"); 0
    })"#,
    );
    let (_interpreter, result) = run(compile(&text).expect("compile nested catches"));
    let error = caught_error(result);
    // SAFETY: the caught error is a two-field record and field zero is Text.
    unsafe {
        let message = *(error.as_ptr::<u8>().add(heap::OBJECT_HEADER_SIZE) as *const Value);
        let (bytes, len) = heap::text_record_payload(message.as_ptr()).expect("Text");
        assert_eq!(std::slice::from_raw_parts(bytes, len), b"outer message");
    }
}

#[test]
fn panic_unwind_removes_contexts_and_registers_owned_by_departing_frames() {
    use verum_vbc::instruction::{Instruction, Reg};
    let mut module = compile(&source(
        "core.base.panic",
        "PanicInfo",
        "fence(|| victim())",
    ))
    .expect("compile");
    let message_id = module.intern_string("context panic");
    let start = module.bytecode.len();
    for instruction in [
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 7,
        },
        Instruction::CtxProvide {
            ctx_type: 98765,
            value: Reg(0),
            body_offset: 0,
        },
        Instruction::Panic {
            message_id: message_id.0,
        },
    ] {
        verum_vbc::bytecode::encode_instruction(&instruction, &mut module.bytecode);
    }
    let descriptor = module
        .functions
        .iter_mut()
        .find(|fd| {
            module
                .strings
                .get(fd.name)
                .is_some_and(|name| name.ends_with(".victim"))
        })
        .expect("victim");
    descriptor.bytecode_offset = start as u32;
    descriptor.bytecode_length = (module.bytecode.len() - start) as u32;
    let (interpreter, result) = run(module);
    let _ = caught_error(result);
    assert!(interpreter.state.context_stack.get(98765).is_none());
    assert_eq!(interpreter.state.registers.top(), 0);
}
