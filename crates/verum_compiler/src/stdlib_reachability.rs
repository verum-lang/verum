//! Compute the set of stdlib modules reachable from a user entry point.
//!
//! This is the bridge between the AST (`MountDecl` / `MountTreeKind`) and
//! the pre-computed `stdlib_dep_graph::DepGraph`. The pipeline calls
//! [`compute_reachable_stdlib_modules`] once per user compilation; the
//! result is the minimal set of stdlib modules that need to be parsed +
//! registered for type-checking to succeed.
//!
//! # Algorithm
//!
//!   1. Walk the user `Module` AST, extracting every `mount` statement.
//!      Convert each `MountTree` into one or more **module-path seeds**.
//!   2. Hand the seed set to `DepGraph::reachable_from`, which BFS-walks
//!      the pre-computed mount graph (including parent-chain climbing
//!      for nested-leaf seeds and glob expansion via the index).
//!   3. Return the closed set.
//!
//! # Conservative pruning
//!
//! The walk is deliberately over-approximate:
//!
//!   * `mount core.X.{a, b}` enqueues `core.X`, `core.X.a`, `core.X.b`.
//!     If `a` is an item rather than a module, the runtime resolver
//!     drops it during registration. If `a` is a re-exported module
//!     from a different prefix, the dep graph's transitive closure
//!     covers it via the owning module's edges.
//!   * `mount core.X.*` expands to every module whose path begins with
//!     `core.X.` per the `StdlibModuleIndex`.
//!   * Forward-declared modules and inline `module …` declarations are
//!     left to the existing late-resolution path; this pass does not
//!     attempt to enumerate them.
//!
//! # Performance contract
//!
//!   * Walk over a ~50-statement entry point: <1 ms.
//!   * BFS over the dep graph for typical reachable sets (50-300 nodes):
//!     <2 ms.
//!   * No allocation in the hot path beyond the result `HashSet` and a
//!     small `VecDeque` inside `DepGraph::reachable_from`.

use std::collections::HashSet;

use verum_ast::{Item, ItemKind, Module, MountDecl, MountTree, MountTreeKind, Path, PathSegment};

use crate::stdlib_dep_graph;
use crate::stdlib_index::{self, StdlibModuleIndex};

/// Compute the transitive closure of stdlib modules required to
/// type-check the given user module.
///
/// Returns `None` when the embedded dep graph is unavailable
/// (e.g. minimal builds without `core/`); callers should fall back to
/// the full-load path in that case.
pub fn compute_reachable_stdlib_modules(user: &Module) -> Option<HashSet<String>> {
    let graph = stdlib_dep_graph::get_dep_graph()?;
    let index = stdlib_index::get_module_index()?;

    // Build the seed set: user mounts ⋃ implicit prelude.
    //
    // The prelude must be in the seed set because `core/mod.vr`'s
    // `public mount super.X` re-exports define the symbols that are
    // available without an explicit `mount` (List, Map, Maybe, Result,
    // …). User code may reference these without ever writing `mount
    // core.collections`, so the BFS must seed them unconditionally.
    let mut seeds = collect_user_mount_seeds(user);
    seeds.extend(prelude_seeds(index));

    if seeds.is_empty() {
        return Some(HashSet::new());
    }

    // BFS produces a superset that includes seed-derived parent paths
    // and nested-leaf candidates that may not correspond to any real
    // stdlib module. Filter against the index so the returned set only
    // contains paths the loader can actually resolve to a `.vr` source.
    let candidate = graph.reachable_from(&seeds, |prefix| expand_glob(index, prefix));
    let real: HashSet<String> = candidate
        .into_iter()
        .filter(|m| index.module_to_file(m).is_some())
        .collect();
    Some(real)
}

/// Implicit-prelude seed set — modules whose contents are
/// auto-imported into every user compilation via `core/mod.vr`'s
/// `public mount super.X.{…}` re-export chain.
///
/// This is intentionally not hardcoded. We read `core/mod.vr` once
/// (cached by the index) and parse its `mount super.…` body to
/// discover which trees the prelude exposes. Any future change to the
/// prelude shape automatically flows through without a compiler rebuild.
fn prelude_seeds(index: &StdlibModuleIndex) -> Vec<String> {
    let archive = match crate::embedded_stdlib::get_embedded_stdlib() {
        Some(a) => a,
        None => return Vec::new(),
    };
    let Some(root_src) = index.module_source(archive, "core") else {
        return Vec::new();
    };
    extract_prelude_paths(root_src)
}

/// Extract the module-paths referenced by `public mount super.X` /
/// `mount super.X` declarations inside `core/mod.vr`.
///
/// Lightweight regex-free scanner: line-oriented, handles single-line
/// and multi-line nested `mount super.foo.{a, b, …}` forms. The result
/// is the set of distinct module paths whose canonical form is
/// `core.<rest>` (the `super` prefix is rewritten to `core` since
/// `super` from inside `core/mod.vr` resolves to the same crate root
/// in practice — that's what makes the prelude pattern work).
fn extract_prelude_paths(src: &str) -> Vec<String> {
    // Strip line + block comments so `// public mount super.foo;` in
    // a doc-comment doesn't seed the prelude.
    let stripped = strip_comments(src);
    let mut out: Vec<String> = Vec::new();
    let mut cursor = 0usize;
    let bytes = stripped.as_bytes();

    while cursor < bytes.len() {
        // Find the next `mount` keyword on a token boundary.
        let Some(rel) = stripped[cursor..].find("mount") else { break; };
        let kw_pos = cursor + rel;
        let preceded_ok = kw_pos == 0
            || matches!(bytes[kw_pos - 1], b' ' | b'\t' | b'\n' | b'\r');
        let followed_ok = matches!(bytes.get(kw_pos + 5), Some(b' ' | b'\t' | b'\n'));
        if !preceded_ok || !followed_ok {
            cursor = kw_pos + 5;
            continue;
        }
        let stmt_end = stripped[kw_pos..].find(';').map(|p| kw_pos + p).unwrap_or(stripped.len());
        let body = stripped[kw_pos + 5..stmt_end].trim();
        cursor = stmt_end + 1;

        // Only the `super.…` family is a prelude marker — everything
        // else is a normal stdlib import that the dep graph already
        // covers.
        if !body.starts_with("super.") && body != "super" {
            continue;
        }
        // Rewrite `super.X` → `core.X`.
        let rewritten = body.replacen("super", "core", 1);
        accumulate_prelude_paths(&rewritten, &mut out);
    }
    out
}

fn accumulate_prelude_paths(body: &str, out: &mut Vec<String>) {
    // Drop any `as Alias` clause.
    let body = match body.find(" as ") {
        Some(p) => &body[..p],
        None => body,
    }.trim();

    if let Some(brace_open) = body.find('{') {
        let prefix = body[..brace_open].trim_end_matches('.').trim();
        out.push(prefix.to_string());
        let inner = &body[brace_open + 1..];
        let close = inner.rfind('}').unwrap_or(inner.len());
        let leaves = &inner[..close];
        for leaf in leaves.split(',') {
            let leaf = leaf.trim();
            if leaf.is_empty() { continue; }
            let leaf_head = leaf.split_whitespace().next().unwrap_or("");
            let leaf_head = leaf_head.split('{').next().unwrap_or(leaf_head);
            if leaf_head == "*" || leaf_head.is_empty() {
                continue;
            }
            out.push(format!("{}.{}", prefix, leaf_head));
        }
    } else if let Some(p) = body.strip_suffix(".*") {
        out.push(p.trim().to_string());
    } else {
        let p = body.trim().to_string();
        out.push(p);
    }
}

/// Strip `//` and `/* */` comments while preserving strings (the body
/// of a real `mount … = …` would never contain a quoted `mount`, but
/// doc-comments often do — defensive).
fn strip_comments(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    let mut in_block = false;
    let mut in_string = false;
    let mut in_line = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_line {
            if c == b'\n' { in_line = false; out.push('\n'); }
            i += 1;
            continue;
        }
        if in_block {
            if c == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                in_block = false;
                i += 2;
                continue;
            }
            if c == b'\n' { out.push('\n'); }
            i += 1;
            continue;
        }
        if in_string {
            if c == b'\\' && i + 1 < bytes.len() {
                out.push(c as char); out.push(bytes[i + 1] as char); i += 2; continue;
            }
            if c == b'"' { in_string = false; }
            out.push(c as char); i += 1; continue;
        }
        if c == b'/' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'/' { in_line = true; i += 2; continue; }
            if bytes[i + 1] == b'*' { in_block = true; i += 2; continue; }
        }
        if c == b'"' { in_string = true; }
        out.push(c as char);
        i += 1;
    }
    out
}

/// Walk the user AST and collect every module-path candidate referenced
/// by a `mount` statement. The result is the seed list for the BFS in
/// `DepGraph::reachable_from`.
fn collect_user_mount_seeds(user: &Module) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in user.items.iter() {
        collect_seeds_from_item(item, &mut out);
    }
    // De-duplicate while preserving first-seen order — keeps the BFS
    // deterministic without a HashSet allocation up front.
    let mut seen: HashSet<String> = HashSet::with_capacity(out.len());
    out.retain(|p| seen.insert(p.clone()));
    out
}

fn collect_seeds_from_item(item: &Item, out: &mut Vec<String>) {
    match &item.kind {
        ItemKind::Mount(decl) => collect_from_mount_decl(decl, out),
        // Inline modules can themselves contain `mount` statements.
        ItemKind::Module(module_decl) => {
            if let verum_common::Maybe::Some(items) = &module_decl.items {
                for inner in items.iter() {
                    collect_seeds_from_item(inner, out);
                }
            }
        }
        _ => {}
    }
}

fn collect_from_mount_decl(decl: &MountDecl, out: &mut Vec<String>) {
    walk_tree("", &decl.tree, out);
}

fn walk_tree(prefix: &str, tree: &MountTree, out: &mut Vec<String>) {
    match &tree.kind {
        MountTreeKind::Path(p) => {
            push_with_parent(combine(prefix, p), out);
        }
        MountTreeKind::Glob(p) => {
            // For globs we record the source module itself so the BFS
            // glob expander walks every submodule under it.
            push_with_parent(combine(prefix, p), out);
        }
        MountTreeKind::Nested { prefix: inner_prefix, trees } => {
            let new_prefix = combine(prefix, inner_prefix);
            // The prefix module itself is needed (its mod.vr re-exports
            // the leaves).
            push_with_parent(new_prefix.clone(), out);
            for sub in trees.iter() {
                walk_tree(&new_prefix, sub, out);
            }
        }
        MountTreeKind::File { .. } => {
            // Relative file mounts are user-cog modules, not stdlib —
            // ignore for the stdlib reachability pass.
        }
    }
}

/// Push a module-path candidate plus every ancestor up to (but NOT
/// including) the `core` root. Including `core` itself would seed the
/// BFS with the whole stdlib via `core/mod.vr`'s glob re-exports — the
/// exact pessimisation this whole pass exists to avoid.
fn push_with_parent(path: String, out: &mut Vec<String>) {
    let mut current: &str = path.as_str();
    let owned = path.clone();
    out.push(owned);
    while let Some(dot) = current.rfind('.') {
        let parent = &current[..dot];
        // Stop one segment above `core`. The owning crate root is
        // implicitly available — no need to drag it into the BFS.
        if parent.is_empty() || parent == "core" { break; }
        out.push(parent.to_string());
        current = parent;
    }
}

/// Concatenate `prefix.path` while normalising leading `.` segments
/// (used by `public mount .submodule.{…}` inside a mod.vr).
fn combine(prefix: &str, p: &Path) -> String {
    let segments: Vec<&str> = p
        .segments
        .iter()
        .filter_map(|seg| match seg {
            PathSegment::Name(ident) => Some(ident.name.as_str()),
            PathSegment::Relative => None, // leading-dot marker
            PathSegment::SelfValue => Some("self"),
            PathSegment::Super => Some("super"),
            PathSegment::Cog => Some("cog"),
        })
        .collect();
    let tail = segments.join(".");
    if prefix.is_empty() {
        tail
    } else if tail.is_empty() {
        prefix.to_string()
    } else {
        format!("{}.{}", prefix, tail)
    }
}

/// Glob expander used by the BFS: enumerate every stdlib module whose
/// path begins with `prefix.`.
fn expand_glob(index: &StdlibModuleIndex, prefix: &str) -> Vec<String> {
    let needle = format!("{}.", prefix);
    let mut out: Vec<String> = Vec::new();
    // Modules are sorted lexicographically — could binary-search the
    // range later. Linear scan is fine for now: 2266 entries × <100 ns
    // string comparison = ~200 µs.
    for m in index.all_modules() {
        if m == prefix || m.starts_with(&needle) {
            out.push(m.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::{
        Ident, MountDecl, MountTree, MountTreeKind, Path, PathSegment, Visibility,
    };
    use verum_ast::span::Span;
    use verum_common::Maybe;

    fn dummy_span() -> Span { Span::dummy() }
    fn ident(s: &str) -> Ident { Ident::new(s, dummy_span()) }
    fn path(parts: &[&str]) -> Path {
        Path {
            segments: parts.iter().map(|p| PathSegment::Name(ident(p))).collect(),
            span: dummy_span(),
        }
    }

    fn user_module_with_mounts(decls: Vec<MountDecl>) -> Module {
        let items: verum_common::List<Item> = decls
            .into_iter()
            .map(|d| Item::new(ItemKind::Mount(d), dummy_span()))
            .collect();
        Module::new(items, verum_ast::FileId::dummy(), dummy_span())
    }

    fn mount_decl(tree_kind: MountTreeKind) -> MountDecl {
        MountDecl {
            visibility: Visibility::Private,
            tree: MountTree {
                kind: tree_kind,
                alias: Maybe::None,
                span: dummy_span(),
            },
            alias: Maybe::None,
            span: dummy_span(),
        }
    }

    #[test]
    fn empty_module_has_empty_seeds() {
        let m = user_module_with_mounts(vec![]);
        let seeds = collect_user_mount_seeds(&m);
        assert!(seeds.is_empty());
    }

    #[test]
    fn path_mount_pushes_parents() {
        // mount core.shell.exec
        let m = user_module_with_mounts(vec![
            mount_decl(MountTreeKind::Path(path(&["core", "shell", "exec"])))
        ]);
        let seeds = collect_user_mount_seeds(&m);
        assert!(seeds.contains(&"core.shell.exec".to_string()));
        assert!(seeds.contains(&"core.shell".to_string()));
        // `core` itself is intentionally NOT seeded — including it would
        // cascade through `core/mod.vr` re-exports and pull in the whole
        // stdlib.
        assert!(!seeds.contains(&"core".to_string()));
    }

    #[test]
    fn nested_mount_walks_each_leaf() {
        // mount core.shell.{exec, jobs}
        let nested = MountTreeKind::Nested {
            prefix: path(&["core", "shell"]),
            trees: vec![
                MountTree { kind: MountTreeKind::Path(path(&["exec"])), alias: Maybe::None, span: dummy_span() },
                MountTree { kind: MountTreeKind::Path(path(&["jobs"])), alias: Maybe::None, span: dummy_span() },
            ].into(),
        };
        let m = user_module_with_mounts(vec![mount_decl(nested)]);
        let seeds = collect_user_mount_seeds(&m);
        assert!(seeds.contains(&"core.shell.exec".to_string()));
        assert!(seeds.contains(&"core.shell.jobs".to_string()));
        assert!(seeds.contains(&"core.shell".to_string()));
    }

    #[test]
    fn glob_mount_records_prefix() {
        // mount core.shell.*
        let m = user_module_with_mounts(vec![
            mount_decl(MountTreeKind::Glob(path(&["core", "shell"])))
        ]);
        let seeds = collect_user_mount_seeds(&m);
        assert!(seeds.contains(&"core.shell".to_string()));
    }

    #[test]
    fn end_to_end_reachability_for_shell_exec() {
        // Skip if no embedded stdlib (minimal build).
        let Some(_) = crate::stdlib_dep_graph::get_dep_graph() else { return; };
        let m = user_module_with_mounts(vec![
            mount_decl(MountTreeKind::Path(path(&["core", "shell", "exec"])))
        ]);
        let reachable = compute_reachable_stdlib_modules(&m).unwrap();
        assert!(reachable.contains("core.shell.exec"),
            "should include the directly-mounted module");
        let index = crate::stdlib_index::get_module_index().unwrap();
        // Reachability should be a small fraction of the full stdlib —
        // the whole point of this pass. A regression that cascades to
        // every module (e.g. via an unsuppressed prelude glob) would
        // trip this assertion.
        assert!(reachable.len() < index.len() / 2,
            "reachable {} should be much smaller than total {}",
            reachable.len(), index.len());
    }
}
