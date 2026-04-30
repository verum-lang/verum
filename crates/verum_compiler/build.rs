//! Build script for verum_compiler
//!
//! Embeds the Verum standard library (core/*.vr) into the compiler binary
//! as a zstd-compressed archive. This enables single-binary distribution
//! without external stdlib dependencies.
//!
//! Archive format:
//!   [file_count: u32]
//!   [index: (path_len: u16, path: utf8, content_offset: u32, content_len: u32) × file_count]
//!   [data: concatenated .vr source texts]
//!
//! At runtime, the archive is decompressed once into memory (~2ms for 4.7MB)
//! and provides instant access to all stdlib source files via path lookup.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    // Locate core/ directory (two levels up from crates/verum_compiler/)
    let project_root = Path::new(&manifest_dir).parent().unwrap().parent().unwrap();
    let core_dir = project_root.join("core");

    if !core_dir.exists() {
        // No core/ directory — skip embedding (for CI or minimal builds)
        let archive_path = Path::new(&out_dir).join("stdlib_archive.bin");
        fs::write(&archive_path, &[] as &[u8]).unwrap();
        println!("cargo:rustc-env=STDLIB_ARCHIVE_PATH={}", archive_path.display());
        println!("cargo:warning=core/ directory not found — embedded stdlib disabled");
        return;
    }

    // Collect all .vr files
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    collect_vr_files(&core_dir, &core_dir, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));

    // Build uncompressed archive
    let archive = build_archive(&files);

    // Compress with zstd (level 19 for maximum compression, decompression is still fast)
    let compressed = zstd::encode_all(archive.as_slice(), 19).unwrap();

    // Write to OUT_DIR
    let archive_path = Path::new(&out_dir).join("stdlib_archive.zst");
    fs::write(&archive_path, &compressed).unwrap();

    println!("cargo:rustc-env=STDLIB_ARCHIVE_PATH={}", archive_path.display());
    println!(
        "cargo:warning=Embedded stdlib: {} files, {:.1}KB compressed (from {:.1}KB)",
        files.len(),
        compressed.len() as f64 / 1024.0,
        archive.len() as f64 / 1024.0,
    );

    // === Mount dependency graph =============================================
    //
    // For each stdlib file, extract the set of `mount`/`public mount` paths
    // it references. The graph lets the runtime walk only the modules
    // transitively reachable from a user entry point's mount set, instead
    // of registering all 2266 stdlib modules upfront.
    //
    // Cost at build time: ~150-300ms scan + ~50KB compressed archive.
    // Cost at runtime: ~5ms decompress + ~1ms BFS for typical entry point.
    //
    // The scanner is regex-light (line-oriented match on `mount …;`); the
    // grammar is constrained enough that this avoids dragging the full
    // verum_fast_parser into the build script (which would create a build
    // cycle since this script *belongs* to verum_compiler).
    let dep_archive = build_dep_graph(&files);
    let dep_compressed = zstd::encode_all(dep_archive.as_slice(), 19).unwrap();
    let dep_path = Path::new(&out_dir).join("stdlib_dep_graph.zst");
    fs::write(&dep_path, &dep_compressed).unwrap();
    println!("cargo:rustc-env=STDLIB_DEP_GRAPH_PATH={}", dep_path.display());
    println!(
        "cargo:warning=Stdlib mount graph: {} edges, {:.1}KB compressed (from {:.1}KB)",
        dep_edge_count(&files),
        dep_compressed.len() as f64 / 1024.0,
        dep_archive.len() as f64 / 1024.0,
    );

    // Rerun if any .vr file changes
    println!("cargo:rerun-if-changed={}", core_dir.display());
    for (path, _) in &files {
        println!("cargo:rerun-if-changed={}", core_dir.join(path).display());
    }
}

// =============================================================================
// Mount dependency graph extraction
// =============================================================================
//
// Archive layout (mirrors stdlib_archive but smaller):
//   [module_count: u32]
//   per module:
//     [path_len: u16] [path: utf8]
//     [edge_count: u16]
//     per edge:
//       [edge_kind: u8]   // 0=Path, 1=Glob, 2=Nested-leaf
//       [path_len: u16] [path: utf8]
//
// All paths are pre-normalised to module-path form (`core.shell.exec`).
// The archive is consumed by `crate::stdlib_dep_graph::DepGraph` at runtime.

const EDGE_PATH: u8 = 0;
const EDGE_GLOB: u8 = 1;
const EDGE_NESTED: u8 = 2;

/// Convert a stdlib file-relative path to its canonical module path.
/// Mirrors `crate::stdlib_index::file_path_to_module_path` — the two
/// must stay in sync.
fn file_to_module(rel: &str) -> String {
    let normalised = rel.replace('\\', "/");
    let mut parts: Vec<&str> = vec!["core"];
    for component in normalised.split('/') {
        if component.is_empty() { continue; }
        let trimmed = component.strip_suffix(".vr").unwrap_or(component);
        parts.push(trimmed);
    }
    let joined = parts.join(".");
    joined.strip_suffix(".mod").map(str::to_string).unwrap_or(joined)
}

/// Strip line and block comments so a `mount` token inside a comment
/// isn't picked up as an edge. Kept simple: handles `//` line comments
/// and `/* … */` block comments (non-nested — sufficient for stdlib).
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
            if c == b'\\' && i + 1 < bytes.len() { out.push(c as char); out.push(bytes[i + 1] as char); i += 2; continue; }
            if c == b'"' { in_string = false; }
            out.push(c as char);
            i += 1;
            continue;
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

/// Edge kinds extracted from a single source file.
struct Edges {
    /// `mount core.shell.exec` — load this module directly
    path: Vec<String>,
    /// `mount core.shell.*` — load this module + all its submodules
    glob: Vec<String>,
    /// `mount core.shell.{exec, jobs}` — flattened to per-leaf module paths
    nested: Vec<String>,
}

/// Walk `mount … ;` statements in a single source.
///
/// Only `mount` statements are recorded as edges. `module X;` submodule
/// declarations are deliberately NOT recorded: they declare that a
/// child `X.vr` exists, but importing the parent should NOT pull in
/// every child — that would re-collapse the reachable set to "the whole
/// stdlib" via the implicit parent → children chain in every `mod.vr`.
/// Children that the user actually consumes are reached through the
/// parent's `public mount .child.{Item}` re-exports, which DO appear
/// as edges.
///
/// `current_module` is the module path of the file being scanned. It
/// is currently unused but reserved for future relative-path edges
/// (e.g. resolving a stray `super` reference inside a stdlib module).
fn extract_mounts(src: &str, current_module: &str) -> Edges {
    let stripped = strip_comments(src);
    let mut edges = Edges { path: Vec::new(), glob: Vec::new(), nested: Vec::new() };

    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i + 5 < bytes.len() {
        // Look for "mount" keyword on a token boundary.
        if bytes[i..].starts_with(b"mount") {
            let preceded_ok = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r');
            let followed_ok = matches!(bytes.get(i + 5), Some(b' ' | b'\t' | b'\n'));
            if preceded_ok && followed_ok {
                let stmt_end = stripped[i..].find(';').map(|p| i + p).unwrap_or(stripped.len());
                let body = &stripped[i + 5..stmt_end];
                parse_mount_body(body.trim(), current_module, &mut edges);
                i = stmt_end + 1;
                continue;
            }
        }
        i += 1;
    }
    edges
}

/// Parse the body of a `mount` statement: everything between `mount`
/// and `;`, with leading/trailing whitespace already stripped.
///
/// `current_module` is used to resolve relative-leading-dot imports
/// (`public mount .submodule.{Item}` inside `core/foo/mod.vr` resolves
/// to `core.foo.submodule.Item`). Without this, the relative form
/// drops information and produces bogus root-level edges.
fn parse_mount_body(body: &str, current_module: &str, edges: &mut Edges) {
    // Drop any trailing `as Alias` clause — doesn't affect dep edges.
    let body = match body.find(" as ") {
        Some(p) => &body[..p],
        None => body,
    }.trim();

    if body.starts_with("./") || body.starts_with("../") {
        // Relative file mount — not a stdlib dependency.
        return;
    }

    if let Some(brace_open) = body.find('{') {
        // Nested mount: prefix.{a, b, c, …}
        let prefix_raw = body[..brace_open].trim_end_matches('.').trim();
        let prefix_path = resolve_path(prefix_raw, current_module);
        let inner = &body[brace_open + 1..];
        let close = inner.rfind('}').unwrap_or(inner.len());
        let leaves = &inner[..close];
        for leaf in split_top_level_commas(leaves) {
            let leaf = leaf.trim();
            if leaf.is_empty() { continue; }
            // The leaf may itself be a nested expression — strip aliases
            // and brace groups.
            let leaf_head = leaf.split_whitespace().next().unwrap_or("");
            let leaf_head = leaf_head.split('{').next().unwrap_or(leaf_head);
            if leaf_head == "*" {
                // mount p.{*} — equivalent to glob on prefix
                edges.glob.push(prefix_path.clone());
            } else if !leaf_head.is_empty() {
                // The leaf may be a sub-module *or* an item — we record it
                // as a candidate module path; the runtime resolver will
                // discard non-existent module candidates.
                let candidate = format!("{}.{}", prefix_path, leaf_head);
                edges.nested.push(candidate);
                // Also record the prefix itself — it's the module that
                // owns the items.
                edges.nested.push(prefix_path.clone());
            }
        }
    } else if let Some(p) = body.strip_suffix(".*") {
        let resolved = resolve_path(p, current_module);
        // Suppress prelude-style globs.
        //
        // `mount core.*;` is the canonical "import the implicit prelude"
        // pattern in stdlib + user code (~1019 occurrences). The
        // compiler always preloads the prelude subset, so emitting an
        // explicit edge here would expand to *every* stdlib module and
        // defeat reachability pruning entirely. Identifiers actually
        // referenced via the prelude are resolved by the existing
        // late-resolution path during type-check.
        if !is_prelude_glob(&resolved) {
            edges.glob.push(resolved);
        }
    } else {
        // Plain `mount path`. The path may name a module OR an item
        // within a module — record both the path and its parent.
        let p = resolve_path(body, current_module);
        edges.path.push(p.clone());
        if let Some(dot) = p.rfind('.') {
            edges.path.push(p[..dot].to_string());
        }
    }
}

/// Resolve a possibly-relative mount path to its absolute module path.
///
/// `public mount .list.List` inside `core/collections/mod.vr` (current
/// module = `core.collections`) resolves to `core.collections.list.List`.
/// `public mount super.base.X` inside `core/mod.vr` (current = `core`)
/// resolves to `core.base.X` (the `super` form is rewritten the same
/// way; pragma-level `super` refers to the same crate root).
///
/// Absolute paths (no leading `.` and no `super.`) pass through
/// unchanged.
fn resolve_path(raw: &str, current_module: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix('.') {
        let rest = rest.trim_start_matches('.');
        if rest.is_empty() {
            return current_module.to_string();
        }
        if current_module.is_empty() {
            return rest.to_string();
        }
        return format!("{}.{}", current_module, rest);
    }
    // `super.X` from inside the crate root resolves to `core.X`.
    if let Some(rest) = trimmed.strip_prefix("super.") {
        // Pragmatically: `super` refers to the crate root in stdlib
        // headers, where the current module's root segment is `core`.
        let root = current_module.split('.').next().unwrap_or("core");
        return format!("{}.{}", root, rest);
    }
    if trimmed == "super" {
        let root = current_module.split('.').next().unwrap_or("core");
        return root.to_string();
    }
    trimmed.to_string()
}

/// Split a comma-separated list while respecting brace/paren nesting.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let bytes = s.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth -= 1,
            b',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() { out.push(&s[start..]); }
    out
}

/// Strip leading `.` (as in `public mount .submodule.{…}` from inside a
/// `mod.vr`) and any leading whitespace.
fn normalise_path(p: &str) -> String {
    p.trim().trim_start_matches('.').to_string()
}

/// Whether a glob path should be treated as the implicit prelude (and
/// therefore NOT emitted as a graph edge — see `parse_mount_body`).
///
/// Three forms are recognised:
///   * `core` — `mount core.*` from user code
///   * `super` — `mount super.*` from inside a stdlib module
///   * the empty string — defensive guard; `mount .*;` would be
///     malformed but should not crash the scanner.
fn is_prelude_glob(p: &str) -> bool {
    matches!(p, "core" | "super" | "")
}

fn build_dep_graph(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    // Build module path → edges
    let mut entries: Vec<(String, Edges)> = Vec::with_capacity(files.len());
    for (rel, bytes) in files {
        let module = file_to_module(rel);
        let src = std::str::from_utf8(bytes).unwrap_or("");
        let edges = extract_mounts(src, &module);
        entries.push((module, edges));
    }
    // Sort for deterministic on-disk layout
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = Vec::with_capacity(64 * 1024);
    let module_count: u32 = entries.len().try_into().expect("too many modules for u32");
    out.extend_from_slice(&module_count.to_le_bytes());

    for (module, edges) in &entries {
        write_str(&mut out, module);
        let total: u32 = (edges.path.len() + edges.glob.len() + edges.nested.len()) as u32;
        // u16 should suffice — clamp defensively
        let total: u16 = total.try_into().expect("module has too many mount edges");
        out.extend_from_slice(&total.to_le_bytes());
        for p in &edges.path  { out.push(EDGE_PATH);   write_str(&mut out, p); }
        for p in &edges.glob  { out.push(EDGE_GLOB);   write_str(&mut out, p); }
        for p in &edges.nested{ out.push(EDGE_NESTED); write_str(&mut out, p); }
    }
    out
}

fn dep_edge_count(files: &[(String, Vec<u8>)]) -> usize {
    let mut total = 0;
    for (rel, bytes) in files {
        let module = file_to_module(rel);
        let src = std::str::from_utf8(bytes).unwrap_or("");
        let e = extract_mounts(src, &module);
        total += e.path.len() + e.glob.len() + e.nested.len();
    }
    total
}

fn write_str(out: &mut Vec<u8>, s: &str) {
    let len: u16 = s.len().try_into().expect("path too long for u16");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn collect_vr_files(dir: &Path, root: &Path, files: &mut Vec<(String, Vec<u8>)>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_vr_files(&path, root, files);
            } else if path.extension().is_some_and(|e| e == "vr") {
                let relative = path.strip_prefix(root).unwrap().to_string_lossy().to_string();
                // Normalize to forward slashes for cross-platform consistency
                let relative = relative.replace('\\', "/");
                if let Ok(content) = fs::read(&path) {
                    files.push((relative, content));
                }
            }
        }
    }
}

fn build_archive(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut archive = Vec::new();
    let file_count: u32 = files.len().try_into()
        .expect("too many stdlib files to fit in u32");

    // Header: file count
    archive.extend_from_slice(&file_count.to_le_bytes());

    // Calculate data section offset using checked arithmetic to prevent overflow.
    // path.len() is cast safely via try_into, and the sum uses checked_add.
    let mut index_size = 0u32;
    for (path, _) in files {
        let path_len: u32 = path.len().try_into()
            .unwrap_or_else(|_| panic!("stdlib path too long: {}", path));
        // 2 (path_len field) + path bytes + 4 (offset) + 4 (content_len)
        let entry_size = 2u32.checked_add(path_len)
            .and_then(|s| s.checked_add(4 + 4))
            .expect("stdlib archive index entry too large");
        index_size = index_size.checked_add(entry_size)
            .expect("stdlib archive index too large");
    }
    let data_offset = 4u32.checked_add(index_size)
        .expect("stdlib archive header + index too large"); // after header + index

    // Build index and data
    let mut data_section = Vec::new();
    let mut index_section = Vec::new();

    for (path, content) in files {
        let path_bytes = path.as_bytes();
        let data_section_len: u32 = data_section.len().try_into()
            .expect("stdlib archive data section too large");
        let content_offset = data_offset.checked_add(data_section_len)
            .expect("stdlib archive content offset overflow");
        let content_len: u32 = content.len().try_into()
            .expect("stdlib file content too large");

        // Index entry
        index_section.extend_from_slice(&(path_bytes.len() as u16).to_le_bytes());
        index_section.extend_from_slice(path_bytes);
        index_section.extend_from_slice(&content_offset.to_le_bytes());
        index_section.extend_from_slice(&content_len.to_le_bytes());

        // Data
        data_section.extend_from_slice(content);
    }

    archive.extend_from_slice(&index_section);
    archive.extend_from_slice(&data_section);
    archive
}
