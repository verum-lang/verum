//! Stdlib mount audit — diagnostic for silent FunctionNotFound class.
//!
//! Walks every `core/**/*.vr`, parses it, collects each module's
//! public exports, then cross-checks every `mount super.X.{a, b, c}`
//! / `mount cog.X.{a, b, c}` declaration against the resolved target
//! module's export set.  Mounts pointing at non-existent symbols
//! report a violation.
//!
//! # Why
//!
//! Pre-#301 a typoed mount silently degraded into a runtime
//! `FunctionNotFound`.  After #301 the lenient SKIP path produces a
//! panic-stub with the original error message — actionable but still
//! a runtime failure.  This audit surfaces the same class at CI time.
//!
//! Real bug class historically caught: `mount super.cap_audit.{record_revoke, ...}`
//! pointing at `cap_audit.vr` (which only declares `CapEvent`) when
//! the actual writers live in `cap_audit_ring.vr`.  Closes the
//! `test_compile_stdlib_mem_header` regression in one diagnostic.
//!
//! # Status
//!
//! Marked `#[ignore]` because the stdlib currently carries ~250
//! pre-existing mount-target drifts that need a separate sweep to
//! clean up.  Run on demand with:
//!
//! ```text
//! cargo test -p verum_compiler --test stdlib_mount_audit -- --ignored --nocapture
//! ```
//!
//! Once the existing drift is cleaned up the `#[ignore]` annotation
//! comes off and the audit becomes a CI gate that prevents
//! re-introduction.  Until then the test is run as a tool for
//! periodic stdlib-hygiene sweeps.
//!
//! # Scope
//!
//! Validates `super.<segment>.{...}`, `super.super.<segment>.{...}`,
//! and absolute `core.<segment>...{...}` simple-name mounts.  Probes
//! transitive `public mount X.*` glob re-exports up to 3 hops.  A
//! curated `is_language_builtin` predicate exempts compiler-injected
//! names (Bool / Int / Maybe / Iterator / Display / TypeId / etc.).
//!
//! Out-of-scope (skipped, not failed):
//!
//! * Glob mounts in CONSUMER position (`mount X.*` brings in anything)
//! * File-relative mounts (`mount ./foo.vr`) — file system, not module path
//! * Mounts whose target file doesn't exist — already a parser-time error
//! * `cog.X` mounts — cross-cog references depend on workspace config
//! * Bare relative references — could be local-module shorthand the
//!   loader resolves at link time

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use verum_ast::decl::{ItemKind, MountTreeKind};
use verum_ast::ty::Ident;
use verum_common::Maybe;
use verum_fast_parser::Parser;

/// One module's public surface area: its fully-qualified module path
/// plus the simple names of every `public {fn,type,const,protocol,context}`
/// declaration.  Used as the destination side of mount validation.
struct ModuleSurface {
    /// `core.term.widget.paragraph` or whatever the source declared
    /// in its `module ...;` line.  Stored as a dot-joined string
    /// because the audit only ever does string equality.
    qualified_path: String,
    /// Source file path on disk, kept so failure reports cite the
    /// original `.vr` rather than the synthesized module path.
    source_file: PathBuf,
    /// Simple names of every public top-level declaration.  Variant
    /// constructors are also added (e.g. `Add | Sub | Mul` of
    /// `public type ArithOp` lands `ArithOp`, `Add`, `Sub`, `Mul`).
    /// Mount declarations (`public mount X.Y.Z;`) re-exported from
    /// other modules also count.
    exports: HashSet<String>,
    /// Resolved fully-qualified targets of every `public mount X.*`
    /// glob re-export in this module.  During validation a symbol
    /// not found in direct exports is also probed against each
    /// glob target's exports, transitively.  Without this stdlib's
    /// `core.intrinsics.mod.vr` (which exposes `atomic_*` etc. via
    /// `public mount atomic.*`) would false-positive on every
    /// downstream consumer.
    glob_reexports: Vec<String>,
}

/// One unsatisfied mount lookup — a symbol whose target module
/// declares no matching public export.  Carries enough context to
/// turn a CI failure into a one-line patch.
#[derive(Debug)]
struct MountViolation {
    /// File where the offending `mount` line lives.
    consumer_file: PathBuf,
    /// Resolved target module (dot-joined).
    target_module: String,
    /// Symbol that didn't match any public declaration in the target.
    missing_symbol: String,
    /// First three lexicographically-sorted exports of the target
    /// whose simple name shares ≥3 leading characters with the
    /// missing symbol — Levenshtein-free typo hint.
    near_matches: Vec<String>,
}

#[test]
#[ignore = "diagnostic — stdlib carries ~250 pre-existing mount drifts; run with --ignored for hygiene sweeps"]
fn stdlib_mount_targets_resolve() {
    let core_root = locate_core_root();
    let modules = collect_module_surfaces(&core_root);

    // Build qualified-path → ModuleSurface index.
    let by_path: HashMap<&str, &ModuleSurface> = modules
        .iter()
        .map(|m| (m.qualified_path.as_str(), m))
        .collect();

    // Validate every mount in every module.
    let mut violations: Vec<MountViolation> = Vec::new();
    for module in &modules {
        validate_module_mounts(module, &by_path, &mut violations);
    }

    if violations.is_empty() {
        return;
    }

    let mut report = format!(
        "stdlib mount audit found {} unresolved symbol(s):\n",
        violations.len()
    );
    for v in &violations {
        let near = if v.near_matches.is_empty() {
            String::new()
        } else {
            format!(" — near matches: {}", v.near_matches.join(", "))
        };
        report.push_str(&format!(
            "  {}: mount target `{}` has no public `{}`{}\n",
            v.consumer_file.display(),
            v.target_module,
            v.missing_symbol,
            near
        ));
    }
    panic!("{}", report);
}

/// Walk parents from this crate's manifest dir to find the workspace
/// root (contains `core/`).  Mirrors how the compiler discovers the
/// stdlib at runtime so paths stay consistent with the actual build.
fn locate_core_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while p.pop() {
        let candidate = p.join("core");
        if candidate.join("mod.vr").exists() {
            return candidate;
        }
    }
    panic!(
        "could not locate workspace `core/` from CARGO_MANIFEST_DIR={}",
        env!("CARGO_MANIFEST_DIR")
    );
}

/// Collect every `core/**/*.vr` file's exports surface.  Walks
/// recursively, skips parse errors with a debug print (a few
/// stdlib files use experimental syntax that the parser doesn't
/// fully accept yet — they're tracked separately and the audit must
/// not block on their parser-level issues).
fn collect_module_surfaces(core_root: &Path) -> Vec<ModuleSurface> {
    let mut surfaces: Vec<ModuleSurface> = Vec::new();
    walk_vr_files(core_root, &mut |file| {
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut parser = Parser::new(&source);
        let module = match parser.parse_module() {
            Ok(m) => m,
            Err(_) => return,
        };

        let qualified_path = match extract_module_path(&module) {
            Some(p) => p,
            None => return,
        };
        let mut exports: HashSet<String> = HashSet::new();
        let mut glob_reexports: Vec<String> = Vec::new();
        let module_segs: Vec<&str> = qualified_path.split('.').collect();
        for item in module.items.iter() {
            collect_public_exports(item, &mut exports, &module_segs, &mut glob_reexports);
        }

        surfaces.push(ModuleSurface {
            qualified_path,
            source_file: file.to_path_buf(),
            exports,
            glob_reexports,
        });
    });
    surfaces
}

fn walk_vr_files(dir: &Path, visit: &mut dyn FnMut(&Path)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_vr_files(&path, visit);
        } else if path.extension().and_then(|s| s.to_str()) == Some("vr") {
            visit(&path);
        }
    }
}

/// Extract the `module X.Y.Z;` declaration's qualified path from the
/// AST.  The parser flattens dot-separated segments into a single
/// `Ident.name` (e.g., `Ident { name: "core.term.widget.paragraph" }`),
/// so this just hands that name back as the canonical lookup key.
/// Returns `None` for files with no module declaration (legacy stdlib
/// files predating the explicit module discipline).
fn extract_module_path(module: &verum_ast::Module) -> Option<String> {
    for item in module.items.iter() {
        if let ItemKind::Module(mod_decl) = &item.kind {
            let name = mod_decl.name.name.to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// Collect simple names of every `public` top-level item that a
/// downstream `mount` could reasonably target.  Also collects
/// glob-re-export targets so transitive `public mount X.*` chains
/// can be resolved at validation time without requiring the audit
/// to compute the whole transitive surface upfront.
fn collect_public_exports(
    item: &verum_ast::Item,
    out: &mut HashSet<String>,
    module_segs: &[&str],
    glob_reexports: &mut Vec<String>,
) {
    match &item.kind {
        ItemKind::Function(func) if matches!(func.visibility, verum_ast::decl::Visibility::Public) => {
            out.insert(func.name.name.to_string());
        }
        ItemKind::Type(ty_decl)
            if matches!(ty_decl.visibility, verum_ast::decl::Visibility::Public) =>
        {
            out.insert(ty_decl.name.name.to_string());
            // Variant constructors are independently mountable —
            // `mount foo.{Add, Sub}` for `public type ArithOp is Add | Sub;`
            // is a legitimate stdlib idiom.
            if let verum_ast::decl::TypeDeclBody::Variant(variants) = &ty_decl.body {
                for variant in variants.iter() {
                    out.insert(variant.name.name.to_string());
                }
            }
        }
        ItemKind::Const(c) if matches!(c.visibility, verum_ast::decl::Visibility::Public) => {
            out.insert(c.name.name.to_string());
        }
        ItemKind::Protocol(p) if matches!(p.visibility, verum_ast::decl::Visibility::Public) => {
            out.insert(p.name.name.to_string());
        }
        ItemKind::Context(c) if matches!(c.visibility, verum_ast::decl::Visibility::Public) => {
            out.insert(c.name.name.to_string());
        }
        // Public mount re-exports — `public mount X.{a, b};` adds a
        // and b to this module's surface from the perspective of any
        // other module mounting from here.
        // Glob re-exports `public mount X.*;` register the target
        // module path; validation probes each transitively.
        ItemKind::Mount(mount_decl)
            if matches!(mount_decl.visibility, verum_ast::decl::Visibility::Public) =>
        {
            collect_mount_simple_names(&mount_decl.tree, out);
            collect_glob_reexport_targets(&mount_decl.tree, &[], module_segs, glob_reexports);
        }
        // Meta declarations (Verum's macro equivalent) can be mounted too.
        ItemKind::Meta(m) if matches!(m.visibility, verum_ast::decl::Visibility::Public) => {
            out.insert(m.name.name.to_string());
        }
        // Proof items — `public axiom X(...) -> Bool;`,
        // `public theorem`, `public lemma`, `public corollary` —
        // are first-class mountable names; downstream modules pull
        // them in to chain proofs.
        ItemKind::Axiom(a) if matches!(a.visibility, verum_ast::decl::Visibility::Public) => {
            out.insert(a.name.name.to_string());
        }
        ItemKind::Theorem(t) | ItemKind::Lemma(t) | ItemKind::Corollary(t)
            if matches!(t.visibility, verum_ast::decl::Visibility::Public) =>
        {
            out.insert(t.name.name.to_string());
        }
        _ => {}
    }
}

fn collect_mount_simple_names(
    tree: &verum_ast::decl::MountTree,
    out: &mut HashSet<String>,
) {
    match &tree.kind {
        MountTreeKind::Path(path) => {
            if let Some(last) = path.segments.last()
                && let verum_ast::ty::PathSegment::Name(id) = last
            {
                let alias_name = match &tree.alias {
                    Maybe::Some(a) => a.name.to_string(),
                    Maybe::None => id.name.to_string(),
                };
                out.insert(alias_name);
            }
        }
        MountTreeKind::Nested { trees, .. } => {
            for sub in trees.iter() {
                collect_mount_simple_names(sub, out);
            }
        }
        MountTreeKind::Glob(_) | MountTreeKind::File { .. } => {}
    }
}

/// Walk a `public mount` tree and record every glob-re-export's
/// resolved fully-qualified target.  `mount atomic.*` inside
/// `core.intrinsics` resolves to `core.intrinsics.atomic`;
/// `mount super.foo.*` resolves up one level.
fn collect_glob_reexport_targets(
    tree: &verum_ast::decl::MountTree,
    prefix: &[String],
    module_segs: &[&str],
    out: &mut Vec<String>,
) {
    match &tree.kind {
        MountTreeKind::Glob(path) => {
            let segs: Vec<String> = path
                .segments
                .iter()
                .filter_map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.to_string()),
                    _ => None,
                })
                .collect();
            let combined: Vec<String> =
                prefix.iter().cloned().chain(segs).collect();
            if let Some(resolved) = resolve_segs_against_module(&combined, module_segs) {
                out.push(resolved);
            }
        }
        MountTreeKind::Nested { prefix: nested_prefix, trees } => {
            let nested_segs: Vec<String> = nested_prefix
                .segments
                .iter()
                .filter_map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(id) => Some(id.name.to_string()),
                    _ => None,
                })
                .collect();
            let combined: Vec<String> = prefix
                .iter()
                .cloned()
                .chain(nested_segs)
                .collect();
            for sub in trees.iter() {
                collect_glob_reexport_targets(sub, &combined, module_segs, out);
            }
        }
        MountTreeKind::Path(_) | MountTreeKind::File { .. } => {}
    }
}

/// Resolve a relative segment list against a module's qualified
/// path, returning the fully-qualified dot-joined path or `None` if
/// the segments escape the workspace root or hit a `cog`/file form
/// the audit doesn't follow.
fn resolve_segs_against_module(
    segs: &[String],
    module_segs: &[&str],
) -> Option<String> {
    if segs.is_empty() {
        return None;
    }
    let mut up_levels: usize = 0;
    let mut tail_start: usize = 0;
    for (i, s) in segs.iter().enumerate() {
        if s == "super" {
            up_levels += 1;
            tail_start = i + 1;
        } else {
            break;
        }
    }
    if up_levels > 0 {
        if up_levels > module_segs.len() {
            return None;
        }
        let parent = &module_segs[..module_segs.len() - up_levels];
        let tail = &segs[tail_start..];
        let mut resolved: Vec<String> = parent.iter().map(|s| s.to_string()).collect();
        resolved.extend(tail.iter().cloned());
        Some(resolved.join("."))
    } else if segs[0] == "core" {
        Some(segs.join("."))
    } else if segs[0] == "cog" || segs[0].starts_with('.') {
        None
    } else {
        // Bare relative reference — `mount atomic.*` inside
        // `core.intrinsics.mod.vr` resolves to `core.intrinsics.atomic`.
        // Treat as relative to the importing module.
        let mut resolved: Vec<String> =
            module_segs.iter().map(|s| s.to_string()).collect();
        resolved.extend(segs.iter().cloned());
        Some(resolved.join("."))
    }
}

/// Walk one module's mount declarations, resolving each `super.X.{...}`
/// or absolute `core.X.{...}` to the target's `ModuleSurface`, and
/// record every (target, symbol) pair where `symbol` isn't in the
/// target's exports set.
fn validate_module_mounts(
    consumer: &ModuleSurface,
    by_path: &HashMap<&str, &ModuleSurface>,
    violations: &mut Vec<MountViolation>,
) {
    let consumer_segs: Vec<&str> = consumer.qualified_path.split('.').collect();

    let source = match std::fs::read_to_string(&consumer.source_file) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut parser = Parser::new(&source);
    let module = match parser.parse_module() {
        Ok(m) => m,
        Err(_) => return,
    };

    for item in module.items.iter() {
        let mount_decl = match &item.kind {
            ItemKind::Mount(m) => m,
            _ => continue,
        };
        validate_mount_tree(
            &mount_decl.tree,
            &[],
            &consumer_segs,
            consumer,
            by_path,
            violations,
        );
    }
}

fn validate_mount_tree(
    tree: &verum_ast::decl::MountTree,
    prefix: &[&Ident],
    consumer_segs: &[&str],
    consumer: &ModuleSurface,
    by_path: &HashMap<&str, &ModuleSurface>,
    violations: &mut Vec<MountViolation>,
) {
    match &tree.kind {
        MountTreeKind::Nested { prefix: nested_prefix, trees } => {
            // The nested prefix carries every segment up to but not
            // including the leaf — `mount super.X.{a, b}` puts
            // [super, X] here.  Resolve it once and reuse.
            let nested_idents: Vec<&Ident> = nested_prefix
                .segments
                .iter()
                .filter_map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(id) => Some(id),
                    _ => None,
                })
                .collect();
            let mut combined: Vec<&Ident> = prefix.to_vec();
            combined.extend(nested_idents);
            for sub in trees.iter() {
                validate_mount_tree(sub, &combined, consumer_segs, consumer, by_path, violations);
            }
        }
        MountTreeKind::Path(path) => {
            // Simple-path mount: target is everything up to the last
            // segment; symbol is the last segment.
            let segs: Vec<&Ident> = path
                .segments
                .iter()
                .filter_map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(id) => Some(id),
                    _ => None,
                })
                .collect();
            if segs.is_empty() {
                return;
            }
            let mut full: Vec<&Ident> = prefix.to_vec();
            full.extend(segs);
            // Last segment is the symbol; everything before it is the
            // target module path.
            let symbol = full.last().unwrap().name.to_string();
            let target_segs: Vec<&str> = full[..full.len() - 1]
                .iter()
                .map(|id| id.name.as_str())
                .collect();
            check_mount(consumer, consumer_segs, &target_segs, &symbol, by_path, violations);
        }
        MountTreeKind::Glob(_) | MountTreeKind::File { .. } => {}
    }
}

/// Resolve the target path against the consumer module's path
/// (handling `super` / `super.super` traversal), look up the target's
/// exports, and record a violation if the symbol isn't there.
/// Verum's compiler-injected built-in names that can be mounted
/// directly from `core` without any `.vr` declaration.  Sourced from
/// CLAUDE.md's "Semantic Types" mandate (List / Text / Map / Set /
/// Maybe / Heap / Shared) plus the primitive scalar types and a few
/// always-in-scope wrappers (Result, Either) the language privileges.
fn is_language_builtin(name: &str) -> bool {
    matches!(
        name,
        // Primitive scalars
        "Bool"
            | "Int"
            | "Int8" | "Int16" | "Int32" | "Int64" | "Int128"
            | "UInt"
            | "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128"
            | "Float" | "Float32" | "Float64"
            | "Char"
            | "Byte"
            | "Text"
            | "Bytes"
            | "Unit"
            // Short-form numeric aliases routinely mounted under
            // `core.types.{U8, I32, F64, ...}` shorthand — Verum
            // injects these as primitive aliases so consumer
            // ergonomics match Rust / C++ short names.
            | "I8" | "I16" | "I32" | "I64" | "I128"
            | "U8" | "U16" | "U32" | "U64" | "U128"
            | "F16" | "F32" | "F64"
            // Built-in semantic types (CLAUDE.md mandate)
            | "Maybe"
            | "Result"
            | "Either"
            | "List"
            | "Map"
            | "Set"
            | "Deque"
            | "Heap"
            | "Shared"
            | "Box"  // alias accepted for migration ergonomics
            | "Rc" | "Arc"
            // Compiler-injected utility types / protocols
            | "Self"
            | "Type"
            | "TypeId"
            | "Any"
            | "Ordering"
            | "Iterator"
            | "IntoIterator"
            | "Display"
            | "Debug"
            | "Clone"
            | "Copy"
            | "Eq"
            | "PartialEq"
            | "Ord"
            | "PartialOrd"
            | "Hash"
            | "Default"
    )
}

/// Recursive glob-re-export probe.  When `target` exposes
/// `public mount X.*` (Glob), the imported symbols come from `X`'s
/// surface — try them.  Bounded depth so a misconfigured cycle
/// surfaces as a violation rather than infinite recursion.
fn symbol_resolves_via_glob(
    target: &ModuleSurface,
    symbol: &str,
    by_path: &HashMap<&str, &ModuleSurface>,
    depth: usize,
) -> bool {
    if depth == 0 {
        return false;
    }
    for glob_target in &target.glob_reexports {
        let g = match by_path.get(glob_target.as_str()) {
            Some(g) => g,
            None => continue,
        };
        if g.exports.contains(symbol) {
            return true;
        }
        if symbol_resolves_via_glob(g, symbol, by_path, depth - 1) {
            return true;
        }
    }
    false
}

fn check_mount(
    consumer: &ModuleSurface,
    consumer_segs: &[&str],
    target_segs: &[&str],
    symbol: &str,
    by_path: &HashMap<&str, &ModuleSurface>,
    violations: &mut Vec<MountViolation>,
) {
    if target_segs.is_empty() {
        return;
    }
    // Resolve `super` / `super.super.…` against consumer_segs.
    // `core.term.widget.dialog`'s `super.paragraph` resolves to
    // `core.term.widget.paragraph`.
    let mut up_levels: usize = 0;
    let mut tail_start: usize = 0;
    for (i, seg) in target_segs.iter().enumerate() {
        if *seg == "super" {
            up_levels += 1;
            tail_start = i + 1;
        } else {
            break;
        }
    }
    let resolved_segs: Vec<&str> = if up_levels > 0 {
        if up_levels > consumer_segs.len() {
            // Path escapes the workspace root — not a stdlib concern,
            // skip rather than false-positive.
            return;
        }
        let parent = &consumer_segs[..consumer_segs.len() - up_levels];
        let tail = &target_segs[tail_start..];
        parent.iter().chain(tail.iter()).copied().collect()
    } else if target_segs[0] == "core" {
        // Absolute `core.X.Y` — already canonical.
        target_segs.to_vec()
    } else if target_segs[0] == "cog" {
        // Cross-cog reference; out of scope for stdlib audit.
        return;
    } else {
        // Bare module reference — could be a same-module local mount,
        // a sibling shorthand, or a cog-relative path the loader
        // resolves at link time.  Skip rather than false-positive.
        return;
    };
    let resolved = resolved_segs.join(".");

    // Built-in types are not declared in any `.vr` file but are
    // routinely mounted both directly via `mount core.{Bool, Int,
    // Maybe, ...}` and via deeper-qualified paths like
    // `mount core.text.char.Char` that re-emphasise the primitive's
    // conceptual home.  The compiler injects built-ins into every
    // namespace; the audit honours that uniformly regardless of
    // target depth.
    if is_language_builtin(symbol) {
        return;
    }

    let target = match by_path.get(resolved.as_str()) {
        Some(t) => t,
        None => return, // target module not in the audit set; skip.
    };
    // Direct hit?
    if target.exports.contains(symbol) {
        return;
    }
    // Probe transitive glob re-exports (e.g.
    // `core.intrinsics.mod.vr` exposes `atomic_*` via
    // `public mount atomic.*`).  Up to 3 levels of indirection — any
    // longer chain is suspicious enough to surface as a violation.
    if symbol_resolves_via_glob(target, symbol, by_path, 3) {
        return;
    }
    // Compute near-match hints — prefix-of-≥3 plus same-length ±1.
    let mut near: Vec<&String> = target
        .exports
        .iter()
        .filter(|e| {
            let prefix_len = symbol.len().min(e.len()).min(3);
            prefix_len >= 3 && symbol[..prefix_len] == e[..prefix_len]
        })
        .collect();
    near.sort();
    near.dedup();
    let near_matches: Vec<String> = near.iter().take(3).map(|s| (**s).clone()).collect();

    violations.push(MountViolation {
        consumer_file: consumer.source_file.clone(),
        target_module: resolved,
        missing_symbol: symbol.to_string(),
        near_matches,
    });
}
