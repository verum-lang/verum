//! T1359 — `FfiStructField::nested_layout` is an INDEX INTO THE LAYOUT
//! TABLE, so importing a layout out of an archive must remap it.
//!
//! `import_archive_ffi_layout` cloned the archive's layout and pushed it
//! at a fresh index, under a docstring that said layouts are
//! "self-contained … copies them verbatim". That was true until this
//! task gave a field a cross-reference. Thirty lines above, the FFI
//! SIGNATURE's `return_layout_idx` and `param_layout_indices` are
//! remapped explicitly, for exactly the reason that now applies here.
//!
//! REACHABLE, not hypothetical: a programme calling `File.size()`
//! imports the stdlib archive's FFI symbols; `fstat`'s signature pulls
//! in `DarwinStat`; its four `DarwinTimespec` fields carry an index into
//! the ARCHIVE's table, where the consumer's table holds something else.
//! The nested record is then walked with a foreign layout's offsets, or
//! skipped, and either way the timespecs come back wrong with no
//! diagnostic.
//!
//! THE FIXTURE MUST START NON-EMPTY, and that is the whole point of the
//! first test below. Importing into an empty table maps index 0 to index
//! 0 and 1 to 1 — the bug is invisible, and a gate written that way
//! passes on the broken code. A fixture that cannot fail is worse than
//! no fixture, because it is counted.
//!
//! Runs in the `verum_vbc feature-gated guardrails (codegen)` CI job,
//! step "Every gated test target"
//! (`cargo test --locked -p verum_vbc --features codegen --tests`).
#![cfg(feature = "codegen")]

use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::module::{CType, FfiStructField, FfiStructLayout, VbcModule};

/// An "archive" whose layout table holds, in order: a decoy, the nested
/// record, and the outer record that names it. The nested layout is at
/// index 1 THERE — a number that must not survive the trip unchanged
/// unless the consumer happens to agree, which is what the offset in the
/// consumer's table is arranged to prevent.
fn archive() -> VbcModule {
    let mut m = VbcModule::new("archive".to_string());
    let decoy_name = m.intern_string("Decoy");
    let inner_name = m.intern_string("Inner");
    let outer_name = m.intern_string("Outer");
    let f = m.intern_string("f");

    // 0: a decoy, so "index 1" is not also the first entry.
    m.ffi_layouts.push(FfiStructLayout {
        name: decoy_name,
        size: 8,
        align: 8,
        fields: vec![FfiStructField {
            name: f,
            c_type: CType::I64,
            offset: 0,
            size: 8,
            align: 8,
            nested_layout: None,
        }],
        verum_type: None,
    });
    // 1: the nested record.
    m.ffi_layouts.push(FfiStructLayout {
        name: inner_name,
        size: 16,
        align: 8,
        fields: vec![
            FfiStructField {
                name: f,
                c_type: CType::I64,
                offset: 0,
                size: 8,
                align: 8,
                nested_layout: None,
            },
            FfiStructField {
                name: f,
                c_type: CType::I64,
                offset: 8,
                size: 8,
                align: 8,
                nested_layout: None,
            },
        ],
        verum_type: None,
    });
    // 2: the outer record, whose second field NAMES layout 1.
    m.ffi_layouts.push(FfiStructLayout {
        name: outer_name,
        size: 24,
        align: 8,
        fields: vec![
            FfiStructField {
                name: f,
                c_type: CType::I64,
                offset: 0,
                size: 8,
                align: 8,
                nested_layout: None,
            },
            FfiStructField {
                name: f,
                c_type: CType::StructValue,
                offset: 8,
                size: 16,
                align: 8,
                nested_layout: Some(1),
            },
        ],
        verum_type: None,
    });
    m
}

/// Give the consumer a different number of layouts than the archive, so
/// that a verbatim copy lands the nested index on the WRONG entry.
fn consumer_with_padding(pad: usize) -> VbcCodegen {
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for i in 0..pad {
        let name = crate::intern(&mut cg, &format!("Filler{i}"));
        cg.ffi_layouts_mut().push(FfiStructLayout {
            name,
            size: 4,
            align: 4,
            fields: Vec::new(),
            verum_type: None,
        });
    }
    cg
}

fn intern(cg: &mut VbcCodegen, s: &str) -> verum_vbc::types::StringId {
    verum_vbc::types::StringId(cg.intern_string_for_test(s))
}

#[test]
fn a_nested_layout_index_is_remapped_into_the_consumers_table() {
    let arch = archive();
    // THREE fillers: the archive's nested layout is at 1, so a verbatim
    // copy would point at the consumer's `Filler1` instead.
    let mut cg = consumer_with_padding(3);

    let outer_idx = cg
        .import_archive_ffi_layout(&arch, 2)
        .expect("outer layout imports");

    let layouts = cg.ffi_layouts_mut().clone();
    let outer = &layouts[outer_idx as usize];
    let nested_idx = outer.fields[1]
        .nested_layout
        .expect("the nested reference must survive the import");

    assert_ne!(
        nested_idx, 1,
        "the ARCHIVE's index came through unchanged — in this consumer, 1 is a filler"
    );

    let nested = layouts
        .get(nested_idx as usize)
        .expect("the nested index must be in range for the CONSUMER's table");
    assert_eq!(
        nested.size, 16,
        "the nested index points at the wrong layout (size {}, want 16)",
        nested.size
    );
    assert_eq!(nested.fields.len(), 2);
    assert_eq!(nested.fields[1].offset, 8);
}

/// Four fields naming one nested record must import it ONCE, not four
/// times: `DarwinStat` names `DarwinTimespec` four times, and a table
/// that grows per mention is a slow leak rather than a wrong answer —
/// which is why it needs a gate and not a comment.
#[test]
fn a_nested_layout_named_twice_is_imported_once() {
    let mut arch = archive();
    // Give the outer record a THIRD field naming the same nested layout.
    let f = arch.intern_string("g");
    arch.ffi_layouts[2].fields.push(FfiStructField {
        name: f,
        c_type: CType::StructValue,
        offset: 24,
        size: 16,
        align: 8,
        nested_layout: Some(1),
    });

    let mut cg = consumer_with_padding(2);
    let before = cg.ffi_layouts_mut().len();
    let outer_idx = cg.import_archive_ffi_layout(&arch, 2).expect("imports");
    let after = cg.ffi_layouts_mut().len();

    assert_eq!(
        after - before,
        2,
        "expected the outer plus ONE copy of the nested layout, got {} new entries",
        after - before
    );

    let layouts = cg.ffi_layouts_mut().clone();
    let outer = &layouts[outer_idx as usize];
    assert_eq!(
        outer.fields[1].nested_layout, outer.fields[2].nested_layout,
        "two fields naming the same archive layout must name the same consumer layout"
    );
}

/// An archive is untrusted input. A layout that names itself must
/// terminate — the reservation happens before the fields are walked, so
/// the cycle resolves to the entry already made for it.
#[test]
fn a_self_referential_layout_terminates() {
    let mut arch = archive();
    arch.ffi_layouts[2].fields[1].nested_layout = Some(2); // points at itself

    let mut cg = consumer_with_padding(1);
    let idx = cg
        .import_archive_ffi_layout(&arch, 2)
        .expect("a cyclic layout must import, not hang");

    let layouts = cg.ffi_layouts_mut().clone();
    assert_eq!(
        layouts[idx as usize].fields[1].nested_layout,
        Some(idx),
        "the cycle must resolve to the entry reserved for it"
    );
}
