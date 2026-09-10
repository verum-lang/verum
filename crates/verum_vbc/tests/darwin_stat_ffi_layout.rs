#![cfg(feature = "codegen")]
//! The FFI layout the codegen builds for `DarwinStat` must match the C
//! `struct stat` this platform actually uses.
//!
//! WHY A TEST AND NOT A PROGRAMME. `File.size()` is one observable, and one
//! observable cannot distinguish sixteen fields: the same wrong answer is
//! produced by a wrong OFFSET, a wrong nested SIZE, and a wrong declaration
//! ORDER. Measured under T1192 — `File.size()` returned `675893106`, which
//! is the file's `st_atimespec.tv_nsec`, and that single number was
//! compatible with three different defects at once. This asks the layout
//! builder for all sixteen offsets and compares each to the platform.
//!
//! WHY IT READS THE TREE. The declaration under test is
//! `core/sys/darwin/libsystem.vr`; a test carrying its own COPY of that
//! text pins the copy, and stays green while the file drifts away from it.
//! The record is extracted from the file at test time, so the subject of
//! the assertion is the shipped declaration.
//!
//! THE EXPECTED VALUES are not derived from Verum. They were measured with
//! `clang` + `offsetof` on this platform:
//!
//!     st_dev 0(4)   st_mode 4(2)   st_nlink 6(2)   st_ino 8(8)
//!     st_uid 16(4)  st_gid 20(4)   st_rdev 24(4)
//!     st_atimespec 32(16) st_mtimespec 48(16) st_ctimespec 64(16)
//!     st_birthtimespec 80(16)
//!     st_size 96(8) st_blocks 104(8) st_blksize 112(4)
//!     st_flags 116(4) st_gen 120(4)          sizeof = 144
//!
//! Instrument first written by session verum-23 as a printing probe; the
//! reading-from-the-tree and the assertions are this file's addition — a
//! probe that only prints is a census, and a census cannot fail.

use std::path::PathBuf;

use verum_ast::FileId;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::VbcModule;

/// C offsets on Darwin, measured with `offsetof`. Sizes are the C field
/// sizes, not the Verum slot widths.
const C_LAYOUT: &[(&str, u32)] = &[
    ("st_dev", 0),
    ("st_mode", 4),
    ("st_nlink", 6),
    ("st_ino", 8),
    ("st_uid", 16),
    ("st_gid", 20),
    ("st_rdev", 24),
    ("st_atime", 32),
    ("st_mtime", 48),
    ("st_ctime", 64),
    ("st_birthtime", 80),
    ("st_size", 96),
    ("st_blocks", 104),
    ("st_blksize", 112),
    ("st_flags", 116),
    ("st_gen", 120),
];

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <repo>/crates/verum_vbc.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root above crates/verum_vbc")
        .to_path_buf()
}

/// The text of `public type <name> is { ... };` as the tree declares it.
///
/// Returning None is a REFUSAL at the call site, never a skip: a test that
/// silently passes when it cannot find its subject measures nothing.
fn declaration(src: &str, name: &str) -> Option<String> {
    let head = format!("public type {name} is {{");
    let start = src.find(&head)?;
    let end = src[start..].find("\n};")? + start + 3;
    Some(src[start..end].to_string())
}

fn compile(src: &str) -> VbcModule {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(src, file_id);
    let parser = VerumParser::new();
    let ast = parser.parse_module(lexer, file_id).unwrap_or_else(|e| {
        let m: Vec<String> = e.iter().map(|x| format!("{x}")).collect();
        panic!("parse: {}", m.join("\n"))
    });
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("probe"));
    cg.compile_module(&ast).expect("compile")
}

#[test]
fn darwin_stat_layout_matches_the_platform() {
    let path = repo_root().join("core/sys/darwin/libsystem.vr");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

    let timespec = declaration(&src, "DarwinTimespec")
        .expect("DarwinTimespec not found in core/sys/darwin/libsystem.vr");
    let stat = declaration(&src, "DarwinStat")
        .expect("DarwinStat not found in core/sys/darwin/libsystem.vr");

    // The extern block is what makes the layout get built at all: a record
    // referenced by an FFI signature is the trigger. `&unsafe` on purpose —
    // it is the spelling the tree uses and the one that used to be missed.
    let probe = format!(
        "module p;\n\
         @ffi(\"libSystem.B.dylib\")\n\
         extern {{\n  fn fstat(fd: Int32, buf: &unsafe DarwinStat) -> Int32;\n}}\n\
         {timespec}\n{stat}\n"
    );

    let m = compile(&probe);
    let layout = m
        .ffi_layouts
        .iter()
        .find(|l| m.strings.get(l.name) == Some("DarwinStat"))
        .unwrap_or_else(|| {
            panic!(
                "no DarwinStat layout; {} layout(s) present, names resolve to {:?}",
                m.ffi_layouts.len(),
                m.ffi_layouts
                    .iter()
                    .map(|l| m.strings.get(l.name))
                    .collect::<Vec<_>>()
            )
        });

    // ORDER is checked against the SOURCE, not against the layout.
    //
    // `FfiStructField::name` is a GLOBAL interned field id from the
    // codegen's own field-name table — the module's `strings` is a
    // different table, and looking the id up there returns an unrelated
    // string (measured: field 0 came back as `"probe"`, the module name).
    // The tree already warns about this id at `marshal_verum_struct_to_c`
    // in `ffi/runtime.rs`: it "must never be used as the slot index".
    // Reading it as a NAME is the same misuse one table over.
    //
    // The declaration's order is a property of the SOURCE, so ask the
    // source. That also makes the two assertions independent: order comes
    // from the file, offsets come from the layout builder.
    let declared: Vec<&str> = stat
        .lines()
        .skip(1)
        .filter_map(|l| {
            let t = l.trim();
            t.split(':').next().filter(|n| {
                !n.is_empty()
                    && !t.starts_with("//")
                    && n.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
            })
        })
        .collect();
    let want_names: Vec<&str> = C_LAYOUT.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        declared, want_names,
        "the declaration's field ORDER no longer matches C's `struct stat`. \
         The layout builder derives offsets from the DECLARED order, so a \
         reordering silently repacks every field after the first difference."
    );

    let mut wrong = Vec::new();
    for (i, (name, want)) in C_LAYOUT.iter().enumerate() {
        let field = layout.fields.get(i).unwrap_or_else(|| {
            panic!(
                "layout has {} fields, C has {}: the declaration lost a field",
                layout.fields.len(),
                C_LAYOUT.len()
            )
        });
        if field.offset != *want {
            wrong.push(format!(
                "  {name:<14} want off={want:<4} got off={:<4} (size={}, ctype={:?})",
                field.offset, field.size, field.c_type
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "{} of {} fields sit at the wrong C offset:\n{}\n\
         A wrong offset here is not a crash — it is a plausible number. \
         `File.size()` returned the file's access-time nanoseconds this way.",
        wrong.len(),
        C_LAYOUT.len(),
        wrong.join("\n")
    );
}
