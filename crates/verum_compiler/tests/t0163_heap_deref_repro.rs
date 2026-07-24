//! T0163 / T0147 / T0109 — dereferencing a `Heap<sum-type>` must preserve the
//! variant tag.
//!
//! `&*Heap<P>` reaches the interpreter's match/`is` tag-read as the Heap CBGR
//! cell's DATA pointer. Before the fix, the tag was read at
//! `data_ptr + OBJECT_HEADER_SIZE`, which lands inside the boxed Value / padding
//! and yields ~0 — so EVERY `&*Heap<P>` read as the first variant (`Zero`).
//! `core.concurrency.process.alpha_eq` recurses into `Heap<Process>` bodies via
//! `&*ba`; every `Send` body read as `Zero`, so `(Zero,Zero)=>true` and all
//! recursive process comparisons short-circuited to true (T0163's symptom).
//!
//! The fix peels the Heap CBGR cell before the tag read (mirroring
//! `handle_deref`'s `cbgr_allocations` arm) in both `handle_match_tag` sites in
//! `pattern_matching.rs`. Runs through the exact `verum run` path
//! (`compile_script_to_vbc`, stdlib-linked so `Heap` resolves).
use std::sync::Arc;
use verum_vbc::interpreter::Interpreter;

fn run_main_i64(vbc: verum_vbc::VbcModule) -> i64 {
    let m = Arc::new(vbc);
    // Same entry resolution `verum run` (ScriptEngine::run) uses.
    let func_id = m
        .find_function_by_name("main")
        .or_else(|| m.find_function_by_unique_bare_suffix("main"))
        .expect("main entry not found");
    let mut interp = Interpreter::new(m);
    let result = interp
        .execute_function_with_args(func_id, &[])
        .expect("execution failed");
    result.try_as_i64().expect("main did not return an Int")
}

/// A Heap-boxed `Send` (tag 1), deref'd and checked with `is` — must be `Send`,
/// not `Zero` (tag 0). Pre-fix this returned 0; the fix returns 3.
const T0163_SRC: &str = "type P is Zero | Send { c: Text }; \
    fn main() -> Int { let h: Heap<P> = Heap(P.Send { c: Text.from(\"x\") }); \
    let r = &*h; if r is P.Send { 3 } else { 0 } }";

#[test]
fn t0163_heap_deref_preserves_variant_tag() {
    let vbc = verum_compiler::api::compile_script_to_vbc(T0163_SRC)
        .expect("compile_script_to_vbc failed");
    assert_eq!(
        run_main_i64(vbc),
        3,
        "&*Heap<P> boxing P.Send read as variant tag 0 (Zero) — Heap-deref tag bug (T0163/T0147/T0109)"
    );
}
