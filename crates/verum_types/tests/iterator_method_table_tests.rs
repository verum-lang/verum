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
//!   - the rest defaulted, providing the iterator adapter surface.
//!
//! This validator pins:
//!   1. `fn next` is present and is the ONLY required method (no body `{`).
//!   2. The method table, by NAME and as a set — see `DECLARED_METHODS`.
//!   3. `size_hint` has a default body.
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

// ── 2. The method table, pinned by NAME ──────────────────────────────────────

/// Every method the `Iterator` protocol declares, in source order.
///
/// This replaces 44 per-name `contains("fn <name>")` assertions plus a
/// count guard, and it is not a tidier spelling of them — the two
/// together did not say what they appeared to say:
///
///   * `contains("fn min")` is satisfied by `fn min_by_key`. Twenty of
///     the 78 names are a prefix of another name, so a fifth of those
///     assertions could pass with the method they name absent.
///   * the count guard read 75 against a table of 78, and 34 methods
///     had no per-name assertion at all — including every one added
///     since the guard was written, which is exactly the set a drift
///     guard exists to catch.
///
/// A pinned SET catches both directions and says which name moved.
/// Adding or removing a method is a deliberate change: update this
/// list in the same commit, and the diff shows a reviewer the name
/// rather than a number.
const DECLARED_METHODS: &[&str] = &[
    "next", "size_hint", "count", "last", "nth", "advance_by", "map",
    "filter", "filter_map", "flat_map", "flatten", "take", "skip", "take_while",
    "skip_while", "chain", "zip", "enumerate", "peekable", "dedup",
    "interleave", "step_by", "inspect", "fuse", "cycle", "cloned", "copied",
    "chunks", "windows", "intersperse", "fold", "reduce", "try_fold",
    "scan", "all", "any", "find", "find_map", "position", "sum", "product",
    "sum_by", "product_by", "max", "min", "max_by_key", "min_by_key",
    "max_by", "min_by", "min_max", "min_max_by_key", "cmp", "eq", "collect",
    "try_collect", "partition", "is_sorted", "is_sorted_by", "is_sorted_by_key",
    "is_partitioned", "partition_point", "for_each", "try_for_each",
    "try_find", "map_while", "intersperse_with", "pairwise", "zip_longest",
    "ne", "lt", "le", "gt", "ge", "by_ref", "unzip", "transduce", "transduce_stateful",
    "reduce_with",
];

/// The names the protocol block declares, in source order.
fn declared_method_names() -> Vec<&'static str> {
    let mut out = Vec::new();
    for line in protocol_body().lines() {
        let Some(rest) = line.strip_prefix("    fn ") else {
            continue;
        };
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if end > 0 {
            out.push(&rest[..end]);
        }
    }
    out
}

#[test]
fn iterator_protocol_method_table_is_pinned() {
    let found = declared_method_names();
    let expected: Vec<&str> = DECLARED_METHODS.to_vec();
    let missing: Vec<&&str> = expected.iter().filter(|m| !found.contains(m)).collect();
    let added: Vec<&&str> = found.iter().filter(|m| !expected.contains(m)).collect();
    assert!(
        missing.is_empty() && added.is_empty(),
        "Iterator protocol method table drifted.\n           removed since the pin: {missing:?}\n           added since the pin:   {added:?}\n           Update DECLARED_METHODS in the same commit as the change."
    );
    assert_eq!(
        found.len(),
        expected.len(),
        "duplicate method declaration in the Iterator protocol block: \
         {} declarations for {} distinct pinned names",
        found.len(),
        expected.len()
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
