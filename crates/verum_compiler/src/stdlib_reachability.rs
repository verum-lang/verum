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

    let seeds = collect_user_mount_seeds(user);
    if seeds.is_empty() {
        // No `mount` statements at all — nothing reachable from user
        // perspective. Return an empty set; callers may decide to fall
        // back to a minimal preload (e.g. just `core` itself).
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
