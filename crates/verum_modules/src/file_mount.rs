//! File-relative mount resolver (#5 / P1.5).
//!
//! Recursively walks `MountTreeKind::File` declarations across
//! a set of already-parsed AST modules, loads each referenced
//! `.vr` file via [`ModuleLoader::load_file_mount`], parses it,
//! and emits a complete list of newly-registered modules ready
//! for the existing module-registry pipeline.
//!
//! ## Architectural shape
//!
//! The pipeline already handles module-path mounts
//! (`mount foo.bar`) end-to-end: parser produces
//! `MountTreeKind::Path`, the loader resolves the dotted path
//! to a file via the cog-root walk, the registry indexes the
//! parsed module by its module path, and the import-resolution
//! pass walks the registry.
//!
//! File mounts (`mount ./helper.vr;`) plug into the same
//! pipeline by **synthesising a module path** for each loaded
//! file: alias if the mount carries one, else the file's
//! basename without `.vr`. The loaded module becomes
//! addressable as a regular module under that synthetic name.
//!
//! Concretely:
//!
//! ```text
//! // src/main.vr
//! mount ./helper.vr;
//!
//! // src/helper.vr
//! module helper;
//! public fn ping() -> Int { 42 }
//! ```
//!
//! After resolution, the registry contains:
//!  * `main` (parsed from `src/main.vr`)
//!  * `helper` (parsed from `src/helper.vr`, synthesised
//!  from the file basename)
//!
//! and any `helper.ping()` call inside `main` resolves
//! through the existing module-path machinery.
//!
//! ## Cycle protection
//!
//! Recursion is bounded by `MAX_MOUNT_DEPTH = 16` to prevent
//! pathological circular file-mount chains from looping
//! forever. A diamond (two files both mounting a shared
//! third) is fine — the loader's per-file cache dedups
//! repeated reads.
//!
//! ## Determinism
//!
//! The result list is sorted by synthetic module name so
//! downstream registration produces stable ModuleId values
//! across runs.

use std::path::{Path, PathBuf};

use verum_ast::ItemKind;
use verum_ast::decl::MountTreeKind;
use verum_common::Text;

use crate::error::{ModuleError, ModuleResult};
use crate::loader::{ModuleLoader, ModuleSource};

const MAX_MOUNT_DEPTH: usize = 16;

/// One resolved file-mount entry.
#[derive(Debug, Clone)]
pub struct ResolvedFileMount {
    /// Synthetic module name under which this file is
    /// registered (alias if the mount specified `as Name`,
    /// else the file basename without `.vr`).
    pub synthetic_name: String,
    /// Absolute resolved path to the loaded `.vr` file.
    pub absolute_path: PathBuf,
    /// Source contents of the loaded file.
    pub source: Text,
    /// Parsed once during traversal and transferred to the registry.
    pub module: verum_ast::Module,
    /// Stable FileId allocated by the loader.
    pub file_id: verum_ast::FileId,
}

impl ResolvedFileMount {
    /// Construct from a `ModuleSource` produced by
    /// `loader.load_file_mount`.
    fn from_source(
        synthetic_name: String,
        source: ModuleSource,
        module: verum_ast::Module,
    ) -> Self {
        Self {
            synthetic_name,
            module,
            absolute_path: source.file_path,
            source: source.source,
            file_id: source.file_id,
        }
    }
}

/// Walk a starting set of `(path, ast_module)` pairs,
/// resolve every transitively reachable
/// `MountTreeKind::File` declaration through `loader`, and
/// return the deduplicated list of files to register as
/// modules.
///
/// The starting set is the user's already-parsed entry
/// points (cog root .vr files); the returned list excludes
/// those entries so callers can simply concat both lists
/// into the registry.
///
/// `parse_one` is a caller-supplied callback that converts
/// a raw `ModuleSource` into a `verum_ast::Module`. The
/// callback shape (rather than a hard dep on
/// verum_fast_parser) keeps this module dep-light — callers
/// that want a custom parser can plug it in.
pub fn resolve_file_mounts<'a, P>(
    loader: &mut ModuleLoader,
    seeds: impl IntoIterator<Item = &'a (PathBuf, verum_ast::Module)>,
    mut parse_one: P,
) -> ModuleResult<Vec<ResolvedFileMount>>
where
    P: FnMut(&ModuleSource) -> ModuleResult<verum_ast::Module>,
{
    let mut output: Vec<ResolvedFileMount> = Vec::new();
    // Only edges enter the worklist. In particular an ordinary dotted mount
    // never needs a filesystem lookup or a copy of its containing syntax tree.
    let mut frontier = Vec::new();
    let mut seed_paths = Vec::new();
    for (path, ast) in seeds {
        seed_paths.push(path);
        let edges = file_mount_edges(ast);
        if !edges.is_empty() {
            frontier.push((path.clone(), edges, 0));
        }
    }
    if frontier.is_empty() {
        return Ok(output);
    }
    let mut visited: std::collections::HashSet<_> = seed_paths
        .into_iter()
        .filter_map(|path| std::fs::canonicalize(path).ok())
        .collect();
    while let Some((importing_path, edges, depth)) = frontier.pop() {
        if depth >= MAX_MOUNT_DEPTH {
            return Err(ModuleError::Other {
                message: Text::from(format!(
                    "file-mount chain exceeded depth {} starting at `{}`",
                    MAX_MOUNT_DEPTH,
                    importing_path.display()
                )),
                span: None,
            });
        }
        for (rel_path, synthetic_name) in edges {
            let source = loader.load_file_mount(rel_path.as_str(), &importing_path)?;
            if !visited.insert(source.file_path.clone()) {
                continue;
            }
            let parsed = parse_one(&source)?;
            frontier.push((
                source.file_path.clone(),
                file_mount_edges(&parsed),
                depth + 1,
            ));
            output.push(ResolvedFileMount::from_source(
                synthetic_name,
                source,
                parsed,
            ));
        }
    }

    // Deterministic order — sort by synthetic_name so
    // downstream ModuleId allocation is stable across runs.
    output.sort_by(|a, b| a.synthetic_name.cmp(&b.synthetic_name));
    Ok(output)
}

fn file_mount_edges(module: &verum_ast::Module) -> Vec<(Text, String)> {
    module
        .items
        .iter()
        .filter_map(|item| {
            let ItemKind::Mount(decl) = &item.kind else {
                return None;
            };
            let MountTreeKind::File { path, .. } = &decl.tree.kind else {
                return None;
            };
            Some((path.clone(), synth_module_name(decl, path.as_str())))
        })
        .collect()
}

/// Derive a synthetic module name for a file mount. Alias
/// wins; otherwise the file basename (without `.vr`).
///
/// The basename fallback uses the LAST path component, so
/// `../shared/util.vr` becomes `util`. Callers that want
/// path-disambiguated names should specify an explicit alias.
fn synth_module_name(decl: &verum_ast::decl::MountDecl, rel_path: &str) -> String {
    if let verum_common::Maybe::Some(alias) = &decl.alias {
        return alias.name.as_str().to_string();
    }
    let base = Path::new(rel_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown_file_mount");
    base.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::FileId;
    use verum_fast_parser::VerumParser;
    use verum_lexer::Lexer;

    fn parse_module_text(text: &str, file_id: FileId) -> verum_ast::Module {
        let lexer = Lexer::new(text, file_id);
        let parser = VerumParser::new();
        parser
            .parse_module(lexer, file_id)
            .unwrap_or_else(|errs| panic!("parse failed: {:?}", errs))
    }

    #[test]
    fn resolves_single_file_mount_to_basename() {
        let root = tempfile::TempDir::new().unwrap();
        let main_path = root.path().join("main.vr");
        let helper_path = root.path().join("helper.vr");
        std::fs::write(&main_path, "module main;\nmount ./helper.vr;\n").unwrap();
        std::fs::write(
            &helper_path,
            "module helper;\npublic fn ping() -> Int { 42 }\n",
        )
        .unwrap();

        let mut loader = ModuleLoader::new(root.path());
        let main_ast = parse_module_text(
            &std::fs::read_to_string(&main_path).unwrap(),
            FileId::new(1),
        );

        let resolved = resolve_file_mounts(&mut loader, &[(main_path.clone(), main_ast)], |src| {
            Ok(parse_module_text(src.source.as_str(), src.file_id))
        })
        .expect("resolution should succeed");

        assert_eq!(resolved.len(), 1, "expected exactly one resolved file");
        assert_eq!(resolved[0].synthetic_name, "helper");
        assert!(resolved[0].absolute_path.ends_with("helper.vr"));
    }

    #[test]
    fn alias_wins_over_basename() {
        let root = tempfile::TempDir::new().unwrap();
        let main = root.path().join("main.vr");
        let helper = root.path().join("helper.vr");
        std::fs::write(&main, "module main;\nmount ./helper.vr as Util;\n").unwrap();
        std::fs::write(&helper, "module helper;\n").unwrap();

        let mut loader = ModuleLoader::new(root.path());
        let main_ast = parse_module_text(&std::fs::read_to_string(&main).unwrap(), FileId::new(1));

        let resolved = resolve_file_mounts(&mut loader, &[(main, main_ast)], |src| {
            Ok(parse_module_text(src.source.as_str(), src.file_id))
        })
        .unwrap();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].synthetic_name, "Util");
    }

    #[test]
    fn diamond_pattern_dedupes_shared_file() {
        // a.vr mounts shared.vr; b.vr also mounts shared.vr.
        // The shared file must appear exactly once in the
        // resolved list.
        let root = tempfile::TempDir::new().unwrap();
        let a = root.path().join("a.vr");
        let b = root.path().join("b.vr");
        let shared = root.path().join("shared.vr");
        std::fs::write(&a, "module a;\nmount ./shared.vr;\n").unwrap();
        std::fs::write(&b, "module b;\nmount ./shared.vr;\n").unwrap();
        std::fs::write(&shared, "module shared;\n").unwrap();

        let mut loader = ModuleLoader::new(root.path());
        let a_ast = parse_module_text(&std::fs::read_to_string(&a).unwrap(), FileId::new(1));
        let b_ast = parse_module_text(&std::fs::read_to_string(&b).unwrap(), FileId::new(2));

        let resolved = resolve_file_mounts(&mut loader, &[(a, a_ast), (b, b_ast)], |src| {
            Ok(parse_module_text(src.source.as_str(), src.file_id))
        })
        .unwrap();

        // Shared file appears exactly once despite two
        // call sites referencing it.
        assert_eq!(resolved.len(), 1, "diamond pattern must dedupe shared file");
        assert_eq!(resolved[0].synthetic_name, "shared");
    }

    #[test]
    fn transitive_mounts_are_followed() {
        // a.vr → mid.vr → leaf.vr
        let root = tempfile::TempDir::new().unwrap();
        let a = root.path().join("a.vr");
        let mid = root.path().join("mid.vr");
        let leaf = root.path().join("leaf.vr");
        std::fs::write(&a, "module a;\nmount ./mid.vr;\n").unwrap();
        std::fs::write(&mid, "module mid;\nmount ./leaf.vr;\n").unwrap();
        std::fs::write(&leaf, "module leaf;\n").unwrap();

        let mut loader = ModuleLoader::new(root.path());
        let a_ast = parse_module_text(&std::fs::read_to_string(&a).unwrap(), FileId::new(1));

        let resolved = resolve_file_mounts(&mut loader, &[(a, a_ast)], |src| {
            Ok(parse_module_text(src.source.as_str(), src.file_id))
        })
        .unwrap();

        // Both transitive mounts must surface, sorted
        // alphabetically by synthetic name.
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].synthetic_name, "leaf");
        assert_eq!(resolved[1].synthetic_name, "mid");
    }

    #[test]
    fn missing_file_mount_surfaces_clean_error() {
        let root = tempfile::TempDir::new().unwrap();
        let a = root.path().join("a.vr");
        std::fs::write(&a, "module a;\nmount ./missing.vr;\n").unwrap();

        let mut loader = ModuleLoader::new(root.path());
        let a_ast = parse_module_text(&std::fs::read_to_string(&a).unwrap(), FileId::new(1));

        let result = resolve_file_mounts(&mut loader, &[(a, a_ast)], |src| {
            Ok(parse_module_text(src.source.as_str(), src.file_id))
        });
        assert!(result.is_err());
    }
}
