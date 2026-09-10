//! T1359 — a record-typed FIELD inside an FFI struct must be laid out
//! at its real size, and the layout must not depend on hash order.
//!
//! Two defects lived in the FFI layout builder, and neither could be
//! seen until T1192 made `&unsafe <record>` reach it at all:
//!
//! * `ctype_size_align` answered `(0, 1)` for `CType::StructValue` — a
//!   nested record occupied ZERO bytes, so every field after it sat at
//!   the wrong offset and the struct was short by the whole nested
//!   payload. Measured on `DarwinStat` (four `DarwinTimespec` fields,
//!   64 bytes of C timespecs): `st_size` landed at offset 32 where the
//!   platform puts it at 96, so reading it returned
//!   `st_atimespec.tv_sec` — a Unix timestamp presented as a file size,
//!   with no diagnostic of any kind.
//!
//! * `pregenerate_ffi_struct_layouts` iterated a `HashSet`, whose order
//!   Rust randomises per process. Whether the NESTED type's layout
//!   existed yet when the OUTER type was sized was therefore decided by
//!   the hasher's seed: nested-first gave `CType::StructValue`,
//!   outer-first fell to `_ => CType::Ptr`, and the two answers are
//!   eight bytes apart per field. One binary, one source file, two
//!   layouts.
//!
//! The record below is the PREFIX of the real Darwin `struct stat`,
//! field for field, so the offsets asserted here are the platform's own
//! (verified with `clang` + `offsetof` on 2026-09-10: st_dev 0,
//! st_mode 4, st_nlink 6, st_ino 8, st_uid 16, st_gid 20, st_rdev 24,
//! st_atimespec 32, st_mtimespec 48 — and st_size 96 in the full
//! struct, which the trimmed record here places at 64 because it stops
//! after two timespecs).
//!
//! WHAT THIS DOES NOT PIN, so nobody credits it with more than it does:
//! the `DarwinStat` DECLARATION's own field order is a separate defect
//! in `core/sys/darwin/libsystem.vr` (T1192's fourth layer), not in
//! this builder. A correct builder given a wrongly ordered declaration
//! still produces a struct C cannot read.
//!
//! Runs in the `verum_vbc feature-gated guardrails (codegen)` CI job,
//! whose step "Every gated test target" is
//! `cargo test --locked -p verum_vbc --features codegen --tests` — a
//! whole-directory invocation, so this file is covered by existing, not
//! by being added to a list (T1349).
#![cfg(feature = "codegen")]

use verum_ast::FileId;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::{CType, FfiStructLayout, VbcModule};

/// The C-visible prefix of `struct stat`, with the nested timespecs
/// that are the whole point of this gate.
const STAT_PREFIX_FIELDS: &str = "\
    st_dev: Int32,\n\
    st_mode: UInt16,\n\
    st_nlink: UInt16,\n\
    st_ino: UInt64,\n\
    st_uid: UInt32,\n\
    st_gid: UInt32,\n\
    st_rdev: Int32,\n\
    st_atime: ProbeTimespec,\n\
    st_mtime: ProbeTimespec,\n\
    st_size: Int64,\n";

/// What a C compiler produces for exactly those fields, in that order,
/// under natural alignment.
const EXPECTED_STAT_OFFSETS: &[(&str, u32, u16)] = &[
    ("st_dev", 0, 4),
    ("st_mode", 4, 2),
    ("st_nlink", 6, 2),
    ("st_ino", 8, 8),
    ("st_uid", 16, 4),
    ("st_gid", 20, 4),
    ("st_rdev", 24, 4),
    ("st_atime", 32, 16),
    ("st_mtime", 48, 16),
    ("st_size", 64, 8),
];
const EXPECTED_STAT_SIZE: u32 = 72;

fn parse(source: &str) -> verum_ast::Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    parser.parse_module(lexer, file_id).unwrap_or_else(|errs| {
        let msgs: Vec<String> = errs.iter().map(|e| format!("{}", e)).collect();
        panic!("parse failed:\n{}", msgs.join("\n"))
    })
}

fn compile(source: &str) -> VbcModule {
    let ast = parse(source);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("t1359"));
    codegen
        .compile_module(&ast)
        .unwrap_or_else(|e| panic!("compile failed: {}", e))
}

/// `nested_first` decides the DECLARATION order of the two records —
/// the source-order half of the hazard the `HashSet` iteration was the
/// other half of.
fn source(nested_first: bool) -> String {
    let timespec = "type ProbeTimespec is { tv_sec: Int, tv_nsec: Int };\n";
    let stat = format!("type ProbeStat is {{\n{STAT_PREFIX_FIELDS}}};\n");
    let decls = if nested_first {
        format!("{timespec}{stat}")
    } else {
        format!("{stat}{timespec}")
    };
    format!(
        "module t;\n\
         \n\
         @ffi(\"libSystem.B.dylib\")\n\
         extern {{\n\
         \x20   fn probe_fstat(fd: Int32, buf: &unsafe ProbeStat) -> Int32;\n\
         }}\n\
         \n\
         {decls}"
    )
}

fn layout<'a>(module: &'a VbcModule, name: &str) -> &'a FfiStructLayout {
    module
        .ffi_layouts
        .iter()
        .find(|l| module.strings.get(l.name) == Some(name))
        .unwrap_or_else(|| {
            // Report the COUNT before the names. The names come out of
            // `module.strings`, which is the very lookup T1360 broke —
            // a "have []" built the same way says "no layouts exist"
            // when it means "no layout NAME resolves", and those are
            // opposite diagnoses. Ask the list its length first.
            let have: Vec<Option<&str>> = module
                .ffi_layouts
                .iter()
                .map(|l| module.strings.get(l.name))
                .collect();
            panic!(
                "no FFI layout named `{name}`; {} layout(s) present, names resolve to {have:?}",
                module.ffi_layouts.len()
            )
        })
}

fn field_names(module: &VbcModule, l: &FfiStructLayout) -> Vec<String> {
    // `FfiStructField::name` is a GLOBAL interned FIELD id, not an index
    // into `module.strings` — see the note in `marshal_verum_struct_to_c`.
    // Positions are what identify a field here, so report the index when
    // the name is unavailable rather than pretend to have one.
    l.fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            module
                .strings
                .get(f.name)
                .map(str::to_string)
                .unwrap_or_else(|| format!("#{i}"))
        })
        .collect()
}

/// The nested record must get a layout of its own, and it must be the
/// one C would build.
#[test]
fn a_nested_record_gets_its_own_c_layout() {
    let module = compile(&source(true));
    let ts = layout(&module, "ProbeTimespec");

    assert_eq!(ts.size, 16, "ProbeTimespec is two 8-byte ints");
    assert_eq!(ts.align, 8);
    assert_eq!(ts.fields.len(), 2);
    assert_eq!(ts.fields[0].offset, 0);
    assert_eq!(ts.fields[1].offset, 8);
    assert_eq!(ts.fields[0].c_type, CType::I64);
}

/// The heart of it: a record-typed field is sized from the nested
/// layout, and every offset after it is the platform's.
#[test]
fn a_record_typed_field_is_sized_and_every_offset_after_it_is_cs() {
    let module = compile(&source(true));
    let st = layout(&module, "ProbeStat");

    assert_eq!(
        st.fields.len(),
        EXPECTED_STAT_OFFSETS.len(),
        "field count changed; layout is {:?}",
        field_names(&module, st)
    );

    for (i, (name, want_off, want_size)) in EXPECTED_STAT_OFFSETS.iter().enumerate() {
        let got = &st.fields[i];
        assert_eq!(
            got.offset, *want_off,
            "field {i} (`{name}`): offset {} but C puts it at {want_off}",
            got.offset
        );
        assert_eq!(
            got.size, *want_size,
            "field {i} (`{name}`): size {} but C gives it {want_size}",
            got.size
        );
    }

    assert_eq!(
        st.size, EXPECTED_STAT_SIZE,
        "struct size {} but C gives {EXPECTED_STAT_SIZE}",
        st.size
    );
    assert_eq!(st.align, 8);
}

/// The exact pre-fix failure, asserted as itself rather than implied by
/// the offsets above: a zero-width nested field.
#[test]
fn a_record_typed_field_is_never_zero_width() {
    let module = compile(&source(true));
    let st = layout(&module, "ProbeStat");
    let ts_idx = module
        .ffi_layouts
        .iter()
        .position(|l| module.strings.get(l.name) == Some("ProbeTimespec"))
        .expect("ProbeTimespec layout") as u16;

    for (i, name) in [(7usize, "st_atime"), (8usize, "st_mtime")] {
        let f = &st.fields[i];
        assert_eq!(
            f.c_type,
            CType::StructValue,
            "`{name}` should be a struct by value"
        );
        assert_ne!(
            f.size, 0,
            "`{name}` is zero-width — this is the T1359 defect verbatim"
        );
        assert_ne!(
            f.size, 8,
            "`{name}` is pointer-sized — the layout was missing and the \
             C-type mapper fell to `_ => CType::Ptr`"
        );
        assert_eq!(f.size, 16, "`{name}` must be the nested layout's size");
        assert_eq!(
            f.nested_layout,
            Some(ts_idx),
            "`{name}` must name WHICH record it holds, or the marshaller \
             cannot copy it in either direction"
        );
    }
}

/// Source order must not be observable in the output. Pre-fix, whether
/// the nested layout existed yet when the outer one was sized was a
/// hash-order coin flip; this is the deterministic half of that hazard.
#[test]
fn declaration_order_does_not_change_the_layout() {
    let nested_first = compile(&source(true));
    let outer_first = compile(&source(false));

    let a = layout(&nested_first, "ProbeStat");
    let b = layout(&outer_first, "ProbeStat");

    assert_eq!(a.size, b.size, "struct size moved with declaration order");
    assert_eq!(a.align, b.align);
    let a_off: Vec<(u32, u16, CType)> =
        a.fields.iter().map(|f| (f.offset, f.size, f.c_type)).collect();
    let b_off: Vec<(u32, u16, CType)> =
        b.fields.iter().map(|f| (f.offset, f.size, f.c_type)).collect();
    assert_eq!(a_off, b_off, "field layout moved with declaration order");
}

/// Repeated compiles in one process share a hash seed, so this cannot
/// see the randomisation directly — it pins the OBSERVABLE the fix
/// makes stable, which is that `ffi_layouts` comes out in sorted-name
/// order regardless of how the set was filled.
#[test]
fn layout_generation_order_is_sorted_not_hashed() {
    // Four FFI-referenced records whose sorted order is nothing like
    // their declaration order.
    let src = "module t;\n\
        \n\
        @ffi(\"libSystem.B.dylib\")\n\
        extern {\n\
        \x20   fn f1(a: &unsafe Zeta) -> Int32;\n\
        \x20   fn f2(a: &unsafe Alpha) -> Int32;\n\
        \x20   fn f3(a: &unsafe Mu) -> Int32;\n\
        \x20   fn f4(a: &unsafe Beta) -> Int32;\n\
        }\n\
        \n\
        type Zeta is { z: Int };\n\
        type Alpha is { a: Int };\n\
        type Mu is { m: Int };\n\
        type Beta is { b: Int };\n";

    let module = compile(src);
    let names: Vec<&str> = module
        .ffi_layouts
        .iter()
        .filter_map(|l| module.strings.get(l.name))
        .collect();

    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(
        names, sorted,
        "layouts were generated in hash order, not sorted order — the \
         same source can then produce different layouts on different runs"
    );

    // And the same source compiled again must give the same thing.
    let again = compile(src);
    let names_again: Vec<&str> = again
        .ffi_layouts
        .iter()
        .filter_map(|l| again.strings.get(l.name))
        .collect();
    assert_eq!(names, names_again);
}

/// T1360 — a layout carried into the module keeps a USABLE name.
///
/// `build_module` cloned each `FfiStructLayout` without remapping its
/// `name`, so a codegen string INDEX arrived where the module expects a
/// byte OFFSET. Nothing read the name at runtime (layouts are reached by
/// index), so it cost no behaviour and exactly one thing: the ability to
/// say which layout you are looking at. Every assertion in this file
/// depends on it, which is why it gets its own test rather than being
/// left implicit in their failures.
#[test]
fn a_layout_carried_into_the_module_keeps_its_name() {
    let module = compile(&source(true));

    assert!(
        !module.ffi_layouts.is_empty(),
        "no FFI layouts at all — the pre-pass did not run, which is a \
         different defect from an unresolvable name"
    );

    let resolved: Vec<&str> = module
        .ffi_layouts
        .iter()
        .filter_map(|l| module.strings.get(l.name))
        .collect();

    assert_eq!(
        resolved.len(),
        module.ffi_layouts.len(),
        "{} of {} layout names resolve in the module string table; the \
         rest are codegen indices read as byte offsets (T1360)",
        resolved.len(),
        module.ffi_layouts.len()
    );
    assert!(resolved.contains(&"ProbeStat"), "have {resolved:?}");
    assert!(resolved.contains(&"ProbeTimespec"), "have {resolved:?}");
}
