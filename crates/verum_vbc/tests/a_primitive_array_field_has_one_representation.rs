//! PACKED-FIELD-ONE-REPRESENTATION-1 (T1463) — a record field declared
//! `[T; N]` with a primitive `T` must have ONE runtime representation,
//! chosen by its DECLARED TYPE and not by the syntax of the expression
//! that happened to fill it.
//!
//! Measured before the fix (2026-09-12), three producers of the SAME
//! declared field type, two representations:
//!
//! ```text
//! let mut w: [UInt32; 44] = [0; 44]; K { w: w }   packed buffer
//! K { w: [22, 0, 0, 0] }                          heap List
//! let w = [33, 0, 0, 0]; K { w: w }               heap List
//! ```
//!
//! The READ is one shape for all three — `GetF` then `GetE` — and `GetE`
//! carries no static element geometry, so the AOT lowering classifies the
//! container at runtime over {cell | stamped Pack | unstamped List}. A bare
//! packed buffer is none of those: its first eight DATA bytes are read as a
//! header word, and above the heap floor they are taken for a cell pointer
//! and DEREFERENCED. `RoundKeys128 is { w: [UInt32; 44] }` faulted at
//! `0x2b7e151702b7e1516` — the AES key material itself, `w[0] | w[1] << 32`.
//! Below the floor the same guess picks the unstamped-List arm and returns
//! element 0 from offset 40: garbage, silently.
//!
//! A fourth classifier arm cannot close this. A packed buffer is unstamped
//! BY DESIGN — the FFI byte-buffer contract requires `[Byte; N]` to reach C
//! as a bare data pointer — so the representation must become single-valued
//! instead. For `elem_size != 1` the target is the heap List: every reader
//! already reads it correctly on both tiers and no consumer changes.
//! `[Byte; N]` fields stay packed, because `&mut self.buf[i] as *mut Byte`
//! must be a real byte pointer (PACKED-FIELD-INIT-DYNCOUNT-1, #37).
//!
//! These tests pin the CODEGEN shape, so a future producer that forgets the
//! normalisation fails here rather than in a crypto suite at Tier 1.

#![cfg(feature = "codegen")]

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

fn count_sub_op(instrs: &[Instruction], want: MemSubOpcode) -> usize {
    instrs
        .iter()
        .filter(|i| matches!(i, Instruction::MemExtended { sub_op, .. } if *sub_op == want.to_byte()))
        .count()
}

fn count_new_list(instrs: &[Instruction]) -> usize {
    instrs
        .iter()
        .filter(|i| matches!(i, Instruction::NewList { .. }))
        .count()
}

/// THE SUBJECT. A tracked packed local stored into a `[UInt32; 4]` field is
/// unpacked into a List at the field-init, so the field's representation
/// matches every other producer of the same declared type.
#[test]
fn a_packed_local_is_unpacked_into_the_field_it_fills() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type K is { w: [UInt32; 4] };\n\
         public fn mk(seed: UInt32) -> K {\n\
             let mut w: [UInt32; 4] = [0; 4];\n\
             w[0] = seed;\n\
             K { w: w }\n\
         }\n",
    );
    let instrs = decoded_fn(&vbc, "mk");

    // The local itself stays packed — that is what makes `w[0] = seed` a
    // static store, and it is not what this fix changes.
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::NewTypedArray),
        1,
        "the annotated local must still allocate a packed buffer:\n{:#?}",
        instrs
    );

    // …and the value that reaches the FIELD is a List built by reading that
    // buffer with the declared stride.
    assert_eq!(
        count_new_list(&instrs),
        1,
        "the packed local must be unpacked into a List before SetF:\n{:#?}",
        instrs
    );
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::TypedArrayLoad),
        1,
        "the unpack loop must read the packed source with TypedArrayLoad \
         (never GetE, which is the runtime classifier this avoids):\n{:#?}",
        instrs
    );
}

/// The shorthand `K { w }` is the same producer as `K { w: w }` and must get
/// the same normalisation — a field-init path that only handled the explicit
/// form would leave the shorthand crashing.
#[test]
fn the_field_shorthand_is_normalised_like_the_explicit_form() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type K is { w: [UInt32; 4] };\n\
         public fn mk(seed: UInt32) -> K {\n\
             let mut w: [UInt32; 4] = [0; 4];\n\
             w[0] = seed;\n\
             K { w }\n\
         }\n",
    );
    let instrs = decoded_fn(&vbc, "mk");
    assert_eq!(
        count_new_list(&instrs),
        1,
        "`K {{ w }}` must unpack exactly like `K {{ w: w }}`:\n{:#?}",
        instrs
    );
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::TypedArrayLoad),
        1,
        "the shorthand's unpack loop must use the declared stride:\n{:#?}",
        instrs
    );
}

/// CONTROL — the byte field keeps its PACKED representation. `[Byte; N]`
/// fields exist to be handed to C as a bare data pointer (#37); unpacking
/// them would break the FFI byte-buffer contract, so the normalisation must
/// stop at `elem_size == 1`.
#[test]
fn a_byte_array_field_is_left_packed() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type B is { b: [Byte; 4] };\n\
         public fn mk(v: Byte) -> B {\n\
             let mut w: [Byte; 4] = [0 as Byte; 4];\n\
             w[0] = v;\n\
             B { b: w }\n\
         }\n",
    );
    let instrs = decoded_fn(&vbc, "mk");
    assert_eq!(
        count_new_list(&instrs),
        0,
        "a `[Byte; N]` field must keep the packed buffer the FFI contract \
         requires:\n{:#?}",
        instrs
    );
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::NewByteArray),
        1,
        "the packed byte local must still be allocated packed:\n{:#?}",
        instrs
    );
}

/// CONTROL — a field filled by an array LITERAL already built a List and
/// must not be unpacked a second time. If this ever grows a
/// `TypedArrayLoad`, the normalisation has started reading a List as a
/// packed buffer, which is the mirror image of the defect.
#[test]
fn a_literal_filled_field_is_not_unpacked() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type K is { w: [UInt32; 4] };\n\
         public fn mk() -> K {\n\
             K { w: [1 as UInt32, 2 as UInt32, 3 as UInt32, 4 as UInt32] }\n\
         }\n",
    );
    let instrs = decoded_fn(&vbc, "mk");
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::TypedArrayLoad),
        0,
        "a List-valued field-init must not be read as a packed buffer:\n{:#?}",
        instrs
    );
    assert_eq!(
        count_new_list(&instrs),
        1,
        "the literal still builds exactly one List:\n{:#?}",
        instrs
    );
}

/// CONTROL — an UNTRACKED local (no annotation, so the frontend built a
/// List) must not be unpacked either. The normalisation is keyed on the
/// frontend's packed-local tracking, not on the field declaration alone.
#[test]
fn an_untracked_local_is_not_unpacked() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type K is { w: [UInt32; 4] };\n\
         public fn mk() -> K {\n\
             let w = [1 as UInt32, 2 as UInt32, 3 as UInt32, 4 as UInt32];\n\
             K { w: w }\n\
         }\n",
    );
    let instrs = decoded_fn(&vbc, "mk");
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::TypedArrayLoad),
        0,
        "an untracked local already holds a List:\n{:#?}",
        instrs
    );
}

