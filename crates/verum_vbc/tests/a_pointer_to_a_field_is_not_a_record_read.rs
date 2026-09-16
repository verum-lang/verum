//! FIELD-POINTEE-WIDTH-1 (T1474) — `ptr_read(&p.a)` must read the FIELD,
//! and the width in the emitted instruction is where that is decided.
//!
//! Measured before the fix, `type Pair is { a: Int, b: Int }`:
//!
//! ```text
//! ptr_read(&p.a)  ->  {41, 99}     want 41
//! ptr_read(&p.b)  ->  {99, …}      want 99
//! ```
//!
//! The record, and then the same record read one slot along — so the
//! address was right and the WIDTH was wrong. A tracer in the 0x1A handler
//! said `size=16 regular_ptr=true`: the interpreter was faithfully
//! rebuilding an object from sixteen bytes because that is what it had been
//! told to do. `try_compile_flat_record_rw` asks `infer_expr_type_name` for
//! the pointee and that helper answers the OWNER's type for `&p.a`, so a
//! field read was classified as a two-field record read.
//!
//! The type system disagrees and is right: in the same programme
//! `takes_int_ref(&p.a)` typechecks against `&Int`, `let r: &Int = &p.a` is
//! accepted, and `*r` answers 41. That is what makes this a codegen defect
//! rather than a language one.
//!
//! These pins read the WIDTH OPERAND rather than running anything: the
//! flat-record emission is exactly "a width other than 1/2/4/8", which the
//! handler documents as its own discriminator (T1160). Checking the
//! discriminator is cheaper than checking the answer and cannot be fooled
//! by a value that happens to look plausible.

use verum_ast::FileId;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
use verum_vbc::bytecode::decode_instructions;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::{Instruction, MemSubOpcode};
use verum_vbc::module::VbcModule;

fn parse(source: &str) -> verum_ast::Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).unwrap_or_else(|errs| {
        let msgs: Vec<String> = errs.iter().map(|e| format!("{}", e)).collect();
        panic!("parse failed:\n{}", msgs.join("\n"))
    })
}

fn compile(module_name: &str, source: &str) -> VbcModule {
    let ast = parse(source);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new(module_name));
    codegen
        .compile_module(&ast)
        .unwrap_or_else(|e| panic!("compile of `{}` failed: {}", module_name, e))
}

fn decoded_fn(module: &VbcModule, suffix: &str) -> Vec<Instruction> {
    let func = module
        .functions
        .iter()
        .find(|f| {
            module
                .strings
                .get(f.name)
                .is_some_and(|n| n == suffix || n.ends_with(&format!(".{suffix}")))
        })
        .unwrap_or_else(|| panic!("function `{}` not found in module", suffix));
    let start = func.bytecode_offset as usize;
    let end = start + func.bytecode_length as usize;
    decode_instructions(&module.bytecode[start..end])
        .unwrap_or_else(|e| panic!("decode of `{}` bytecode failed: {:?}", suffix, e))
}

/// Widths carried by every `ptr_read`/`ptr_write` emission in a function.
///
/// The operand layout is `dst:reg, ptr:reg, width:u8` (and, for a
/// flat-record emission only, a trailing type id). Registers are varints,
/// so the width is read by walking them rather than by a fixed offset.
fn rw_widths(instrs: &[Instruction], want: MemSubOpcode) -> Vec<u8> {
    let mut out = Vec::new();
    for i in instrs {
        if let Instruction::MemExtended { sub_op, operands } = i
            && *sub_op == want.to_byte()
        {
            let mut idx = 0usize;
            // two varint registers precede the width
            for _ in 0..2 {
                while idx < operands.len() && operands[idx] & 0x80 != 0 {
                    idx += 1;
                }
                idx += 1;
            }
            if idx < operands.len() {
                out.push(operands[idx]);
            }
        }
    }
    out
}

const PAIR: &str = r#"
mount core.intrinsics.memory.{ptr_read, ptr_write};
type Pair is { a: Int, b: Int };
"#;

/// THE SUBJECT. A pointer taken at a field is a SCALAR read: width 8, the
/// shape `PtrReadRaw` emits, never the flat-record width.
#[test]
fn a_read_through_a_field_pointer_is_scalar_width() {
    let src = format!(
        "{PAIR}
fn probe() -> Int {{
    let p = Pair {{ a: 41, b: 99 }};
    unsafe {{ ptr_read(&p.a) }}
}}"
    );
    let m = compile("field_read", &src);
    let widths = rw_widths(&decoded_fn(&m, "probe"), MemSubOpcode::PtrRead);
    assert!(!widths.is_empty(), "no ptr_read emitted at all");
    for w in &widths {
        assert!(
            matches!(w, 1 | 2 | 4 | 8),
            "a field pointer was read at width {w}; a width outside 1/2/4/8 is \
             the flat-RECORD emission, which rebuilds the owner instead of \
             answering the field"
        );
    }
}

/// The SECOND field, because reading the first can be right by accident:
/// offset zero coincides with the record's own address.
#[test]
fn a_read_through_a_pointer_to_the_second_field_is_scalar_width() {
    let src = format!(
        "{PAIR}
fn probe() -> Int {{
    let p = Pair {{ a: 41, b: 99 }};
    unsafe {{ ptr_read(&p.b) }}
}}"
    );
    let m = compile("field_read_2", &src);
    let widths = rw_widths(&decoded_fn(&m, "probe"), MemSubOpcode::PtrRead);
    assert!(!widths.is_empty(), "no ptr_read emitted at all");
    for w in &widths {
        assert!(matches!(w, 1 | 2 | 4 | 8), "second field read at width {w}");
    }
}

/// The WRITE twin. A write that lands one slot off looks like a success at
/// the field it was aimed at, so the emission is pinned on its own.
#[test]
fn a_write_through_a_field_pointer_is_scalar_width() {
    let src = format!(
        "{PAIR}
fn probe() {{
    let mut p = Pair {{ a: 41, b: 99 }};
    unsafe {{ ptr_write(&mut p.a, 7); }}
}}"
    );
    let m = compile("field_write", &src);
    let widths = rw_widths(&decoded_fn(&m, "probe"), MemSubOpcode::PtrWrite);
    assert!(!widths.is_empty(), "no ptr_write emitted at all");
    for w in &widths {
        assert!(matches!(w, 1 | 2 | 4 | 8), "field write at width {w}");
    }
}

/// THE CONTROL, AND WHY IT ASSERTS THE GUARD RATHER THAN THE WIDE PATH.
///
/// The control this file wanted is "a pointer to the WHOLE record keeps the
/// wide emission", so that a fix making every `ptr_read` scalar could not
/// pass unnoticed. It could not be written here: the flat-record emission
/// does not fire in this harness for any spelling tried, and — measured —
/// it does not fire on the UNFIXED baseline either, so the pin was failing
/// for its own reason rather than reporting on the fix. The runtime check
/// that replaced it found that `ptr_read(&p)` dies on a null dereference
/// today, independently of this change; that is filed as its own task.
///
/// What IS assertable here is the guard's own shape: it declines on a
/// syntactic `&<field>`, so an argument that is NOT a field address must
/// still reach the width-classifying code below it. A single-field record
/// exercises that, because `try_compile_flat_record_rw` returns early for
/// `field_count < 2` — reaching that early return proves the guard did not
/// swallow the call first.
#[test]
fn a_single_field_record_still_reaches_the_width_classifier() {
    let src = r#"
mount core.intrinsics.memory.{ptr_read};
type Solo is { only: Int };
fn probe() -> Int {
    let s = Solo { only: 7 };
    unsafe { ptr_read(&s.only) }
}"#;
    let m = compile("solo_read", src);
    let widths = rw_widths(&decoded_fn(&m, "probe"), MemSubOpcode::PtrRead);
    assert_eq!(
        widths,
        vec![8],
        "a single-field record's field read must emit the scalar width once"
    );
}
