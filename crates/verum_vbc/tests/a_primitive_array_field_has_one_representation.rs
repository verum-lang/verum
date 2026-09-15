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

/// PACKED-BYTE-FIELD-INDEX-1 (T1463). A `[Byte; N]` field indexed WHERE IT
/// LIVES must use the byte-strided opcodes, not the generic `GetE`/`SetE`
/// whose Tier-1 lowering classifies its receiver at runtime.
///
/// The boundary was measured, not assumed — three shapes of the same field,
/// and only two of them are the defect:
///
/// ```text
/// first(&s.b)   passed as a `&[Byte; 4]` parameter     rc=0
/// k.b[0]        indexed through a `&Struct`            rc=139
/// self.buf[i]   indexed in place inside a method       rc=139
/// ```
///
/// So this is not "byte fields are broken"; it is "a byte field indexed in
/// place is". The whole hash stack dies on the third line: SHA-1/256/512,
/// BLAKE3 and Poly1305 all fault at `0x0` inside their own `update`.
#[test]
fn a_byte_field_indexed_in_place_uses_the_byte_opcodes() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type S is { buf: [Byte; 8], n: Int };\n\
         public fn put(s: &mut S, v: Byte) { s.buf[s.n] = v; }\n\
         public fn get(s: &S) -> Byte { s.buf[s.n] }\n",
    );

    let w = decoded_fn(&vbc, "put");
    assert_eq!(
        count_sub_op(&w, MemSubOpcode::ByteArrayStore),
        1,
        "`s.buf[i] = v` on a declared `[Byte; N]` field must store with the \
         byte stride:\n{:#?}",
        w
    );
    assert!(
        !w.iter().any(|i| matches!(i, Instruction::SetE { .. })),
        "the generic SetE is the classifier path this exists to avoid:\n{:#?}",
        w
    );

    let r = decoded_fn(&vbc, "get");
    assert_eq!(
        count_sub_op(&r, MemSubOpcode::ByteArrayLoad),
        1,
        "`s.buf[i]` on a declared `[Byte; N]` field must load with the byte \
         stride:\n{:#?}",
        r
    );
    assert!(
        !r.iter().any(|i| matches!(i, Instruction::GetE { .. })),
        "the generic GetE is the classifier path this exists to avoid:\n{:#?}",
        r
    );
}

/// CONTROL — a NON-byte primitive array field keeps the generic path. After
/// PACKED-FIELD-ONE-REPRESENTATION-1 such a field holds a heap List with
/// 8-byte Value slots while its DECLARED stride is 4, so a byte-style static
/// stride there would read garbage silently — the exact trade this whole task
/// refuses.
#[test]
fn a_non_byte_array_field_keeps_the_generic_index() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type K is { w: [UInt32; 4], n: Int };\n\
         public fn get(k: &K) -> UInt32 { k.w[k.n] }\n",
    );
    let r = decoded_fn(&vbc, "get");
    assert_eq!(
        count_sub_op(&r, MemSubOpcode::ByteArrayLoad),
        0,
        "a `[UInt32; N]` field must not be read with a byte stride:\n{:#?}",
        r
    );
    assert!(
        r.iter().any(|i| matches!(i, Instruction::GetE { .. })),
        "it keeps the generic index:\n{:#?}",
        r
    );
}

/// CONTROL — a field that is NOT an array is untouched. `packed_field_receiver_type`
/// plus `field_array_spec` must refuse it, or every `s.x[i]` on a List field
/// would be rewritten into a byte load.
#[test]
fn a_list_field_is_not_treated_as_a_packed_array() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type L is { xs: List<Byte>, n: Int };\n\
         public fn get(l: &L) -> Byte { l.xs[l.n] }\n",
    );
    let r = decoded_fn(&vbc, "get");
    assert_eq!(
        count_sub_op(&r, MemSubOpcode::ByteArrayLoad),
        0,
        "a `List<Byte>` field is not a packed array:\n{:#?}",
        r
    );
}

/// A NESTED receiver reaches the same routing (T1463). BLAKE3 is the live
/// instance: its byte buffer is `self.chunk.block_buf`, and while
/// SHA-256/512/1 moved past their `update` on the first field-index fix,
/// `Blake3.update` kept faulting at `0x0` because the receiver of the
/// indexed field was itself a field access.
#[test]
fn a_nested_byte_field_reaches_the_byte_opcodes() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type Inner is { block_buf: [Byte; 64], used: Int };\n\
         public type Outer is { chunk: Inner, n: Int };\n\
         public fn put(o: &mut Outer, v: Byte) { o.chunk.block_buf[o.n] = v; }\n\
         public fn get(o: &Outer) -> Byte { o.chunk.block_buf[o.n] }\n",
    );

    let w = decoded_fn(&vbc, "put");
    assert_eq!(
        count_sub_op(&w, MemSubOpcode::ByteArrayStore),
        1,
        "`o.chunk.block_buf[i] = v` must store with the byte stride:\n{:#?}",
        w
    );
    let r = decoded_fn(&vbc, "get");
    assert_eq!(
        count_sub_op(&r, MemSubOpcode::ByteArrayLoad),
        1,
        "`o.chunk.block_buf[i]` must load with the byte stride:\n{:#?}",
        r
    );
}

/// CONTROL for the nested arm — a nested NON-byte array field keeps the
/// generic index. The recursion must carry the element-size filter down, or
/// `self.chunk.cv[i]` (`[UInt32; 8]`, a List after
/// PACKED-FIELD-ONE-REPRESENTATION-1) would be read with a byte stride.
#[test]
fn a_nested_non_byte_field_keeps_the_generic_index() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type Inner is { cv: [UInt32; 8], used: Int };\n\
         public type Outer is { chunk: Inner, n: Int };\n\
         public fn get(o: &Outer) -> UInt32 { o.chunk.cv[o.n] }\n",
    );
    let r = decoded_fn(&vbc, "get");
    assert_eq!(
        count_sub_op(&r, MemSubOpcode::ByteArrayLoad),
        0,
        "a nested `[UInt32; N]` field must not be read with a byte stride:\n{:#?}",
        r
    );
}

/// PACKED-BYTE-LOCAL-FROM-FIELD-1 (T1463). `let mut buf = self.buf;` with no
/// annotation binds a local to a packed byte buffer; nothing marked it, so
/// every `buf[i]` after it took the generic path. This is where SHA-256 and
/// SHA-512 die today — through `update`, into `finalize`.
#[test]
fn an_unannotated_local_bound_from_a_byte_field_is_packed() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type S is { buf: [Byte; 64], n: Int };\n\
         public fn pad(s: S) -> Byte {\n\
             let mut buf = s.buf;\n\
             buf[0] = 128 as Byte;\n\
             buf[1]\n\
         }\n",
    );
    let instrs = decoded_fn(&vbc, "pad");
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::ByteArrayStore),
        1,
        "the write through the bound local must use the byte stride:\n{:#?}",
        instrs
    );
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::ByteArrayLoad),
        1,
        "and so must the read:\n{:#?}",
        instrs
    );
}

/// CONTROL — a local bound from a NON-byte array field must NOT be marked.
/// Such a field holds a heap List with 8-byte Value slots, so a byte stride
/// would read garbage silently.
#[test]
fn an_unannotated_local_from_a_non_byte_field_is_not_packed() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type K is { w: [UInt32; 8], n: Int };\n\
         public fn first(k: K) -> UInt32 {\n\
             let w = k.w;\n\
             w[0]\n\
         }\n",
    );
    let instrs = decoded_fn(&vbc, "first");
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::ByteArrayLoad),
        0,
        "a `[UInt32; N]` field's local must not be read with a byte stride:\n{:#?}",
        instrs
    );
}

/// CONTROL — an ANNOTATED local keeps its existing route. The annotation path
/// allocates its own packed array (`NewByteArray`); this branch must not
/// double-mark or divert it.
#[test]
fn an_annotated_local_is_unaffected_by_the_field_binding_rule() {
    let vbc = compile(
        "m",
        "module m;\n\
         public fn mk(v: Byte) -> Byte {\n\
             let mut w: [Byte; 4] = [0 as Byte; 4];\n\
             w[0] = v;\n\
             w[0]\n\
         }\n",
    );
    let instrs = decoded_fn(&vbc, "mk");
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::NewByteArray),
        1,
        "the annotated local still allocates its own packed array:\n{:#?}",
        instrs
    );
}

/// PACKED-ARRAY-FROM-CALL-1 (T1475). `let d = make();` where `make` is declared
/// `-> [Byte; N]` binds a local to the callee's PACKED buffer. Nothing marked
/// it, so `d[i]` took the generic path and the Tier-1 classifier read the
/// array's own bytes as a header — the fault address was literally the data
/// (`0x160000000000000b` = 22 and 11).
///
/// 89 functions in `core/` return `[Byte; N]`: every `to_be_bytes`, every
/// `digest`/`finalize`, `hkdf_extract_*`, `chacha20_block`, `uuid.to_bytes`.
#[test]
fn a_local_bound_from_a_call_returning_a_byte_array_is_packed() {
    let vbc = compile(
        "m",
        "module m;\n\
         public fn make() -> [Byte; 8] {\n\
             let mut o: [Byte; 8] = [0 as Byte; 8];\n\
             o[0] = 11 as Byte;\n\
             o\n\
         }\n\
         public fn use_it() -> Byte {\n\
             let d = make();\n\
             d[0]\n\
         }\n",
    );
    let instrs = decoded_fn(&vbc, "use_it");
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::ByteArrayLoad),
        1,
        "reading the returned array must use the byte stride:\n{:#?}",
        instrs
    );
    assert!(
        !instrs.iter().any(|i| matches!(i, Instruction::GetE { .. })),
        "the generic GetE is the classifier path this avoids:\n{:#?}",
        instrs
    );
}

/// The same for a NON-byte return: `[UInt32; 8]` is packed too — measured in
/// the callee's bytecode (`NewTypedArray`, stride 4) — so it wants the typed
/// stride, not the byte one and not the generic path.
#[test]
fn a_local_bound_from_a_call_returning_a_typed_array_is_packed() {
    let vbc = compile(
        "m",
        "module m;\n\
         public fn make32() -> [UInt32; 8] {\n\
             let mut o: [UInt32; 8] = [0; 8];\n\
             o[0] = 11;\n\
             o\n\
         }\n\
         public fn use_it() -> UInt32 {\n\
             let d = make32();\n\
             d[0]\n\
         }\n",
    );
    let instrs = decoded_fn(&vbc, "use_it");
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::TypedArrayLoad),
        1,
        "reading a returned `[UInt32; N]` must use the declared stride:\n{:#?}",
        instrs
    );
}

/// CONTROL — a call returning a `List` must NOT be marked packed. The control
/// matters: the return-form probe measured `List<Byte>` as CORRECT at both
/// tiers already, so marking it would break something that works.
#[test]
fn a_local_bound_from_a_call_returning_a_list_is_not_packed() {
    let vbc = compile(
        "m",
        "module m;\n\
         public fn make_list() -> List<Byte> {\n\
             let mut o: List<Byte> = [];\n\
             o.push(11 as Byte);\n\
             o\n\
         }\n\
         public fn use_it() -> Byte {\n\
             let d = make_list();\n\
             d[0]\n\
         }\n",
    );
    let instrs = decoded_fn(&vbc, "use_it");
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::ByteArrayLoad),
        0,
        "a List return must keep the generic index:\n{:#?}",
        instrs
    );
}

/// A QUALIFIED call reaches the same routing (T1475). `Sha512.digest(…)` is
/// how 89 of the 89 byte-array returns in `core/` are actually spelled —
/// `digest`, `finalize`, `to_be_bytes` are all impl methods — so a lookup by
/// the path's LAST segment covered almost none of them.
///
/// Worse than useless: a bare `"digest"` is first-wins across the tree, so it
/// can resolve to an unrelated function and answer with ITS return type.
#[test]
fn a_local_bound_from_a_qualified_call_is_packed() {
    let vbc = compile(
        "m",
        "module m;\n\
         public type H is { n: Int };\n\
         implement H {\n\
             public fn digest(v: Int) -> [Byte; 8] {\n\
                 let mut o: [Byte; 8] = [0 as Byte; 8];\n\
                 o[0] = 11 as Byte;\n\
                 o\n\
             }\n\
         }\n\
         public fn use_it() -> Byte {\n\
             let d = H.digest(1);\n\
             d[0]\n\
         }\n",
    );
    let instrs = decoded_fn(&vbc, "use_it");
    assert_eq!(
        count_sub_op(&instrs, MemSubOpcode::ByteArrayLoad),
        1,
        "a qualified call's returned array must use the byte stride:\n{:#?}",
        instrs
    );
}
