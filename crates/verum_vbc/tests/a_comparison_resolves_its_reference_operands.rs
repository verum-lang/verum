//! `==` and `<` resolve their operands before reading them (T1491).
//!
//! `match self.last { Maybe.Some(ref l) if l == &x => … }` is the guard
//! `DedupIter` is written with, and it answered TRUE for every element:
//! `dedup()` kept only the first element of any input, including an
//! input with no duplicates at all.
//!
//! `ref l` lowers to `GetVariantDataRef`, a heap-INTERIOR pointer at the
//! payload SLOT; `&x` is a CBGR register-ref. `handle_eqg` took both RAW
//! out of the registers and probed the interior pointer's bytes as an
//! `ObjectHeader` to pick a type to dispatch on — the bytes at a payload
//! slot are not a header, so it read `type_id 1` = `Bool`, looked up
//! `Bool.eq` by a linear scan over the module's functions, and called the
//! first match. The guard's result was a two-field TUPLE (`{1, None}`,
//! the shape of `Iterator.size_hint`), which `if` then read as true.
//!
//! WHY THIS PIN IS STRUCTURAL RATHER THAN BEHAVIOURAL, stated because
//! the behavioural form was written first and DISCARDED: a synthetic
//! module has no colliding `Bool.eq` to dispatch to, so `handle_eqg`
//! falls through to `deep_value_eq`, which peels correctly on its own.
//! The bytecode probe answered `true` / `false` correctly WITH the peel
//! and WITHOUT it — a control that passes both ways proves nothing. The
//! defect needs the archive's function table, and that surface is
//! `vcs/specs/L0-critical/stdlib-runtime/a_guard_comparing_a_ref_binding_answers_a_bool.vr`,
//! which reproduces it end to end.
//!
//! What IS pinnable here is the thing a future edit would undo: that
//! both handlers reach their operands through the resolution authority
//! instead of reading the registers raw.

use std::path::{Path, PathBuf};

fn comparison_rs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("interpreter")
        .join("dispatch_table")
        .join("handlers")
        .join("comparison.rs")
}

fn body_of(src: &str, func: &str) -> String {
    let start = src
        .find(&format!("fn {}(", func))
        .unwrap_or_else(|| panic!("`{}` not found in comparison.rs", func));
    // The operand reads happen in the first 60 lines of either handler;
    // bounding the window keeps a later handler's raw reads out.
    src[start..]
        .lines()
        .take(60)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn both_comparison_handlers_resolve_their_operands() {
    let src = std::fs::read_to_string(comparison_rs()).expect("comparison.rs is readable");

    for func in ["handle_eqg", "handle_cmpg"] {
        let body = body_of(&src, func);
        assert!(
            body.contains("resolve_comparison_operand(state, state.get_reg(a)"),
            "{func} no longer resolves its LEFT operand — a `ref` binding \
             reaches it as a heap-interior pointer and the type probe then \
             reads a payload slot as an ObjectHeader (T1491)"
        );
        assert!(
            body.contains("resolve_comparison_operand(state, state.get_reg(b)"),
            "{func} no longer resolves its RIGHT operand — `&x` over a local \
             reaches it as a CBGR register-ref (T1491)"
        );
    }
}

#[test]
fn the_resolution_authority_is_the_shared_one() {
    // The peel must stay in `cbgr_helpers` beside `resolve_receiver`,
    // not be re-copied into the comparison file: private one-hop copies
    // are how this class of gap keeps reopening (T0705's own note).
    let helpers = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("interpreter")
        .join("dispatch_table")
        .join("handlers")
        .join("cbgr_helpers.rs");
    let src = std::fs::read_to_string(helpers).expect("cbgr_helpers.rs is readable");
    assert!(
        src.contains("fn resolve_comparison_operand"),
        "the comparison-operand peel left cbgr_helpers; it must live \
         beside `resolve_receiver`, which it composes with"
    );
    assert!(
        src.contains("cbgr_mutable_ptrs"),
        "the peel no longer consults `cbgr_mutable_ptrs` — that set is \
         what proves an address is an interior Value slot rather than an \
         object base"
    );
}
