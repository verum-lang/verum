#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Iterator protocol method-table validator (#53).
//!
//! `core/base/iterator.vr` defines the `Iterator` protocol with:
//!   - ONE required method: `fn next(&mut self) -> Maybe<Self.Item>;`
//!   - 74 default methods providing the full iterator adapter surface.
//!
//! This validator pins:
//!   1. `fn next` is present and is the ONLY required method (no body `{`).
//!   2. Core consuming adapters exist: count, last, nth, fold, reduce.
//!   3. Core lazy adapters exist: map, filter, filter_map, flat_map, flatten,
//!      take, skip, take_while, skip_while, chain, zip, enumerate, peekable.
//!   4. Result-aware combinators exist: try_fold, try_collect, try_for_each.
//!   5. Ordering/comparison methods exist: cmp, eq, min, max, sum, product.
//!   6. Utility adapters: for_each, inspect, cycle, step_by, cloned, copied.
//!   7. The total method count in the protocol block is pinned (drift alert).
//!
//! Baking the source in via `include_str!` means file renames and method
//! removals both fail CI immediately.

const ITERATOR_VR: &str = include_str!("../../../core/base/iterator.vr");

fn count_in_protocol_block(method: &str) -> usize {
    // Find the Iterator protocol block boundaries
    let Some(start) = ITERATOR_VR.find("public type Iterator is protocol {") else { return 0 };
    // Find the closing `}` of the protocol (depth-tracking)
    let block_src = &ITERATOR_VR[start..];
    let brace_start = block_src.find('{').unwrap_or(0) + 1;
    let inner = &block_src[brace_start..];
    let mut depth = 1usize;
    let mut end_pos = inner.len();
    for (i, ch) in inner.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end_pos = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let protocol_body = &inner[..end_pos];
    // Count occurrences of method header inside protocol body
    let mut count = 0;
    let mut search_start = 0;
    while let Some(pos) = protocol_body[search_start..].find(method) {
        count += 1;
        search_start += pos + method.len();
    }
    count
}

fn protocol_body() -> &'static str {
    let start = ITERATOR_VR.find("public type Iterator is protocol {").unwrap_or(0);
    let block_src = &ITERATOR_VR[start..];
    let brace_start = block_src.find('{').unwrap_or(0) + 1;
    let inner = &block_src[brace_start..];
    let mut depth = 1usize;
    let mut end_pos = inner.len();
    for (i, ch) in inner.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end_pos = i;
                    break;
                }
            }
            _ => {}
        }
    }
    &inner[..end_pos]
}

fn method_count_in_protocol() -> usize {
    protocol_body().matches("    fn ").count()
}

// ── 1. Required method: next ──────────────────────────────────────────────────

#[test]
fn next_is_required_method_present() {
    assert!(
        ITERATOR_VR.contains("fn next(&mut self) -> Maybe<Self.Item>;"),
        "Iterator must have 'fn next(&mut self) -> Maybe<Self.Item>;' as required method"
    );
}

#[test]
fn next_has_no_default_body() {
    // `fn next` must appear WITHOUT `{` immediately after the signature.
    // If it had a default body it would change the required→default status.
    let body = protocol_body();
    let Some(pos) = body.find("fn next(&mut self) -> Maybe<Self.Item>") else {
        panic!("fn next not found in Iterator protocol body")
    };
    let sig_end = &body[pos..];
    // The very next non-whitespace char after the signature must be `;`, not `{`.
    let after_sig = sig_end.trim_start_matches(|c: char| c != ';' && c != '{');
    assert!(
        after_sig.starts_with(';'),
        "fn next must be a required method (';' not '{{'): found {:?}",
        &after_sig[..after_sig.len().min(20)]
    );
}

// ── 2. Core consuming adapters ────────────────────────────────────────────────

macro_rules! assert_method {
    ($name:literal) => {
        assert!(
            protocol_body().contains(concat!("fn ", $name)),
            concat!("Iterator protocol must contain method '", $name, "'")
        );
    };
}

#[test] fn consuming_count()   { assert_method!("count"); }
#[test] fn consuming_last()    { assert_method!("last"); }
#[test] fn consuming_nth()     { assert_method!("nth"); }
#[test] fn consuming_fold()    { assert_method!("fold"); }
#[test] fn consuming_reduce()  { assert_method!("reduce"); }
#[test] fn consuming_sum()     { assert_method!("sum"); }
#[test] fn consuming_product() { assert_method!("product"); }
#[test] fn consuming_collect() { assert_method!("collect"); }
#[test] fn consuming_for_each() { assert_method!("for_each"); }

// ── 3. Core lazy adapters ─────────────────────────────────────────────────────

#[test] fn adapter_map()          { assert_method!("map"); }
#[test] fn adapter_filter()       { assert_method!("filter"); }
#[test] fn adapter_filter_map()   { assert_method!("filter_map"); }
#[test] fn adapter_flat_map()     { assert_method!("flat_map"); }
#[test] fn adapter_flatten()      { assert_method!("flatten"); }
#[test] fn adapter_take()         { assert_method!("take"); }
#[test] fn adapter_skip()         { assert_method!("skip"); }
#[test] fn adapter_take_while()   { assert_method!("take_while"); }
#[test] fn adapter_skip_while()   { assert_method!("skip_while"); }
#[test] fn adapter_chain()        { assert_method!("chain"); }
#[test] fn adapter_zip()          { assert_method!("zip"); }
#[test] fn adapter_enumerate()    { assert_method!("enumerate"); }
#[test] fn adapter_peekable()     { assert_method!("peekable"); }
#[test] fn adapter_step_by()      { assert_method!("step_by"); }
#[test] fn adapter_cycle()        { assert_method!("cycle"); }
#[test] fn adapter_inspect()      { assert_method!("inspect"); }
#[test] fn adapter_cloned()       { assert_method!("cloned"); }
#[test] fn adapter_copied()       { assert_method!("copied"); }
#[test] fn adapter_map_while()    { assert_method!("map_while"); }

// ── 4. Result-aware combinators ───────────────────────────────────────────────

#[test] fn result_try_fold()       { assert_method!("try_fold"); }
#[test] fn result_try_collect()    { assert_method!("try_collect"); }
#[test] fn result_try_for_each()   { assert_method!("try_for_each"); }
#[test] fn result_try_find()       { assert_method!("try_find"); }

// ── 5. Boolean / search predicates ───────────────────────────────────────────

#[test] fn predicate_all()       { assert_method!("all"); }
#[test] fn predicate_any()       { assert_method!("any"); }
#[test] fn predicate_find()      { assert_method!("find"); }
#[test] fn predicate_find_map()  { assert_method!("find_map"); }
#[test] fn predicate_position()  { assert_method!("position"); }

// ── 6. Ordering / comparison ──────────────────────────────────────────────────

#[test] fn ordering_cmp()        { assert_method!("cmp"); }
#[test] fn ordering_eq()         { assert_method!("eq"); }
#[test] fn ordering_min()        { assert_method!("min"); }
#[test] fn ordering_max()        { assert_method!("max"); }
#[test] fn ordering_min_by_key() { assert_method!("min_by_key"); }
#[test] fn ordering_max_by_key() { assert_method!("max_by_key"); }
#[test] fn ordering_is_sorted()  { assert_method!("is_sorted"); }

// ── 7. Total method count guard ───────────────────────────────────────────────

/// Pins the total number of `fn ` declarations inside the Iterator protocol
/// block.  Adding or removing methods without updating this count is a
/// deliberate, reviewed change — the drift guard prevents silent surface
/// expansion.
#[test]
fn iterator_protocol_method_count_is_75() {
    let count = method_count_in_protocol();
    assert_eq!(
        count, 75,
        "Expected 75 method declarations in Iterator protocol block, got {count}. \
         Update this count after any intentional addition or removal."
    );
}

// ── 8. size_hint default is present ──────────────────────────────────────────

#[test]
fn size_hint_default_method_present() {
    assert!(
        protocol_body().contains("fn size_hint"),
        "Iterator must have 'fn size_hint' default method"
    );
}
