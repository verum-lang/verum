//! Module loading from the filesystem.
//!
//! Loads Verum source files (.vr) and parses them into AST modules.
//! Supports conditional compilation via @cfg attributes.
//!
//! File system mapping rules:
//! 1. `lib.vr` or `main.vr` is the crate root
//! 2. `foo.vr` defines module `foo`
//! 3. `foo/bar.vr` defines module `foo.bar`
//! 4. `foo/mod.vr` defines module `foo` with child modules

use crate::ModuleInfo;
use crate::error::{ModuleError, ModuleResult};
use crate::path::{ModuleId, ModulePath};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use verum_ast::{FileId, Module as AstModule};
use verum_ast::cfg::{CfgEvaluator, TargetConfig};
use verum_common::{List, Map, Maybe, Shared, Text};
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// VUVA #145 / MOD-MED-1 — header-validation diagnostic.
///
/// Two distinct soft-failure modes are surfaced as warnings (build
/// continues, but the user sees a heads-up so they can fix the
/// dangling decl):
///
/// 1. `ForwardDeclNoSource` — `module foo;` (no body) at the top
///    level of a file references a submodule `foo` that has no
///    source file. The forward-decl is structurally valid (Rule 2 /
///    Rule 4 lookup will retry) but if the user expected Rust-style
///    "this file IS module foo" semantics, they're getting silent
///    failure instead.
///
/// 2. `InlineFilesystemOverlap` — `module foo { … }` (inline body)
///    at the top level of a file alongside an existing `<dir>/foo/`
///    directory. The inline `foo` would shadow filesystem-derived
///    `foo` submodules; the user almost certainly didn't intend
///    that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleHeaderWarning {
    /// `module foo;` references a submodule with no source file.
    ForwardDeclNoSource {
        /// The .vr file that contained the forward declaration.
        file: PathBuf,
        /// Name of the forward-declared submodule.
        submodule_name: Text,
        /// Filesystem candidates the loader inspected.
        candidates: List<PathBuf>,
        /// Span of the `module foo;` declaration in `file`.
        span: verum_ast::span::Span,
    },
    /// `module foo { … }` declared inline, but a filesystem `foo/`
    /// directory exists alongside the containing file.
    InlineFilesystemOverlap {
        /// The .vr file that contained the inline declaration.
        file: PathBuf,
        /// Name of the inline submodule.
        submodule_name: Text,
        /// The conflicting directory on disk.
        conflicting_dir: PathBuf,
        /// Span of the `module foo { … }` declaration in `file`.
        span: verum_ast::span::Span,
    },
}

impl ModuleHeaderWarning {
    /// Stable error code for diagnostic emission.
    pub fn code(&self) -> &'static str {
        match self {
            ModuleHeaderWarning::ForwardDeclNoSource { .. } => {
                "E_MODULE_HEADER_FORWARD_DECL_NO_SOURCE"
            }
            ModuleHeaderWarning::InlineFilesystemOverlap { .. } => {
                "E_MODULE_INLINE_FILESYSTEM_OVERLAP"
            }
        }
    }

    /// Human-readable message.
    pub fn message(&self) -> String {
        match self {
            ModuleHeaderWarning::ForwardDeclNoSource {
                file,
                submodule_name,
                candidates,
                ..
            } => {
                let candidate_paths = candidates
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "module '{}' is forward-declared in '{}' but no source file exists \
                     (looked at: {}). \
                     If you meant 'this file IS module {}', remove the forward declaration; \
                     otherwise create the source file.",
                    submodule_name.as_str(),
                    file.display(),
                    candidate_paths,
                    submodule_name.as_str(),
                )
            }
            ModuleHeaderWarning::InlineFilesystemOverlap {
                file,
                submodule_name,
                conflicting_dir,
                ..
            } => format!(
                "inline 'module {} {{ … }}' in '{}' shadows the existing \
                 filesystem directory '{}'. The inline body wins for path \
                 resolution; sibling files under '{}' will be hidden.",
                submodule_name.as_str(),
                file.display(),
                conflicting_dir.display(),
                conflicting_dir.display(),
            ),
        }
    }

    /// Span for diagnostic location.
    pub fn span(&self) -> verum_ast::span::Span {
        match self {
            ModuleHeaderWarning::ForwardDeclNoSource { span, .. } => *span,
            ModuleHeaderWarning::InlineFilesystemOverlap { span, .. } => *span,
        }
    }
}

/// VUVA #145 / MOD-MED-1 — validate module-decl headers in a parsed
/// AST against the filesystem. Returns one warning per header
/// inconsistency found; an empty Vec means the file is clean.
///
/// This is a pure function so it can be unit-tested independently;
/// the loader calls it at parse time and the pipeline drains the
/// list as diagnostics.
pub fn validate_module_headers_against_filesystem(
    file_path: &Path,
    ast: &AstModule,
) -> Vec<ModuleHeaderWarning> {
    let mut warnings = Vec::new();

    // Determine the directory siblings of `file_path`. Submodules
    // live either as `<dir>/<name>.vr` or `<dir>/<name>/mod.vr`.
    // For `lib.vr` / `main.vr` (root files) the sibling dir is the
    // file's parent. For `foo/mod.vr` the sibling dir is the parent
    // of `mod.vr`. For `foo.vr` the sibling dir is the directory of
    // `foo.vr` plus a `<file_stem>/` subtree (a foo.vr can have a
    // foo/ adjacent directory holding `foo.bar.vr` etc.).
    let parent_dir = match file_path.parent() {
        Some(p) => p.to_path_buf(),
        None => return warnings,
    };
    // The "secondary" sibling dir is `<parent>/<file_stem>` —
    // applies to non-mod files only.
    let file_stem = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let is_mod_file = file_stem == "mod";
    let secondary_dir: Option<PathBuf> = if is_mod_file || file_stem.is_empty() {
        None
    } else {
        Some(parent_dir.join(file_stem))
    };

    for item in ast.items.iter() {
        if let verum_ast::ItemKind::Module(module_decl) = &item.kind {
            let submodule_name: Text =
                Text::from(module_decl.name.name.as_str());
            let span = module_decl.span;

            match &module_decl.items {
                Maybe::None => {
                    // Forward decl. Check that at least one of the
                    // expected source files exists.
                    let mut candidates: List<PathBuf> = List::new();
                    let primary_file =
                        parent_dir.join(format!("{}.vr", submodule_name.as_str()));
                    let primary_mod = parent_dir
                        .join(submodule_name.as_str())
                        .join("mod.vr");
                    candidates.push(primary_file.clone());
                    candidates.push(primary_mod.clone());
                    if let Some(sec) = &secondary_dir {
                        let secondary_file =
                            sec.join(format!("{}.vr", submodule_name.as_str()));
                        let secondary_mod = sec
                            .join(submodule_name.as_str())
                            .join("mod.vr");
                        candidates.push(secondary_file);
                        candidates.push(secondary_mod);
                    }
                    let any_exists = candidates.iter().any(|p| p.exists());
                    if !any_exists {
                        warnings.push(ModuleHeaderWarning::ForwardDeclNoSource {
                            file: file_path.to_path_buf(),
                            submodule_name,
                            candidates,
                            span,
                        });
                    }
                }
                Maybe::Some(_) => {
                    // Inline body. Check whether a filesystem
                    // directory of the same name exists adjacent to
                    // the file — that's the overlap case.
                    let primary_dir =
                        parent_dir.join(submodule_name.as_str());
                    let secondary_dir_opt = secondary_dir
                        .as_ref()
                        .map(|s| s.join(submodule_name.as_str()));
                    let overlapping = if primary_dir.exists() && primary_dir.is_dir() {
                        Some(primary_dir)
                    } else if let Some(sec) = secondary_dir_opt {
                        if sec.exists() && sec.is_dir() {
                            Some(sec)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some(conflicting_dir) = overlapping {
                        warnings.push(ModuleHeaderWarning::InlineFilesystemOverlap {
                            file: file_path.to_path_buf(),
                            submodule_name,
                            conflicting_dir,
                            span,
                        });
                    }
                }
            }
        }
    }

    warnings
}

/// Source information for a loaded module.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleSource {
    /// The file path
    pub file_path: PathBuf,
    /// The source code
    pub source: Text,
    /// The file ID
    pub file_id: FileId,
}

impl ModuleSource {
    pub fn new(file_path: PathBuf, source: Text, file_id: FileId) -> Self {
        Self {
            file_path,
            source,
            file_id,
        }
    }
}

/// Module loader - loads modules from the filesystem.
///
/// Implements the file system mapping rules from Section 1.2 of the spec:
/// - `lib.vr` or `main.vr` is the cog root
/// - `foo.vr` defines module `foo`
/// - `foo/bar.vr` defines module `foo.bar`
/// - `foo/mod.vr` defines module `foo` with child modules
///
/// Supports conditional compilation via @cfg attributes:
/// - Module-level: @cfg on module declaration skips entire module
/// - Item-level: @cfg on items filters them during parsing
///
/// Cross-cog resolution: when a CogResolver is attached, the first segment
/// of a module path is checked against registered external cog names.
/// If matched, the module is loaded from the external cog's root directory.
///
/// Supported file extension: `.vr`
#[derive(Debug)]
pub struct ModuleLoader {
    /// Root directory for module search
    root_path: PathBuf,
    /// Name of the cog rooted at `root_path`. When set, any module path
    /// whose first segment equals this name is treated as rooted at
    /// the cog itself — the prefix is stripped before file lookup.
    ///
    /// This is the canonicalisation seam. Without it, the same file
    /// appears under *two* dotted paths: the "absolute" form declared
    /// in the source (`module core.mesh.xds.resources;`) and the
    /// "relative" form derived from the filesystem
    /// (`mesh.xds.resources`, since the loader's root is `core/`).
    /// Both forms now canonicalise to the same key.
    cog_name: Option<String>,
    /// FileId allocator. If a shared counter is attached via
    /// `set_file_id_allocator`, it is used so all loaders sharing the
    /// same compilation session agree on FileId values; otherwise the
    /// local `next_file_id` counter is used (legacy / test mode).
    next_file_id: u32,
    shared_file_id: Option<Shared<AtomicU32>>,
    /// ModuleId allocator. Same shared/local pattern as FileId — but
    /// kept in a *separate* counter because FileId and ModuleId are
    /// different value spaces that were accidentally conflated in
    /// the original impl. Sharing with `ModuleRegistry::next_id`
    /// through `set_module_id_allocator` is how the loader, registry,
    /// and pipeline agree on a single monotonic ModuleId sequence.
    next_module_id: u32,
    shared_module_id: Option<Shared<AtomicU32>>,
    /// Cache of loaded files (by absolute path)
    loaded_files: Map<PathBuf, FileId>,
    /// Canonical ModulePath → stable ModuleId map.
    ///
    /// Without this, every call to `resolve_module(...)` would allocate
    /// a fresh ModuleId for the same canonical module path. Downstream,
    /// `ExportTable::add_export` checks `source_module` equality to
    /// deduplicate re-exports — and if the same type is re-exported
    /// via two `resolve_module` calls, each one carries a distinct
    /// `ModuleId`, so the table sees the export as "same name,
    /// different source" and raises a spurious conflict.
    /// Keyed by the canonical dotted form (e.g. "core.mesh.xds.resources").
    module_path_to_id: Map<String, ModuleId>,
    /// Fully parsed `ModuleInfo` keyed by canonical module path. Served
    /// on repeat `resolve_module` calls so the AST is not re-parsed.
    module_info_cache: Map<String, ModuleInfo>,
    /// Cfg evaluator for conditional compilation
    cfg_evaluator: CfgEvaluator,
    /// External cog resolver for cross-cog imports.
    /// When set, mount paths whose first segment matches a cog name
    /// are dispatched to the cog's root directory.
    cog_resolver: Option<crate::cog_resolver::CogResolver>,
}

impl ModuleLoader {
    /// Create a new module loader with the given root path.
    ///
    /// Uses the host platform's cfg configuration.
    pub fn new(root_path: impl Into<PathBuf>) -> Self {
        Self {
            root_path: root_path.into(),
            cog_name: None,
            next_file_id: 0,
            shared_file_id: None,
            next_module_id: 0,
            shared_module_id: None,
            loaded_files: Map::new(),
            module_path_to_id: Map::new(),
            module_info_cache: Map::new(),
            cfg_evaluator: CfgEvaluator::new(),
            cog_resolver: None,
        }
    }

    /// Attach a shared FileId allocator. All loaders / sessions that
    /// share this counter will hand out a strictly monotonic FileId
    /// sequence. Without this, each loader owns its own counter and
    /// FileIds from different loaders can collide.
    pub fn set_file_id_allocator(&mut self, allocator: Shared<AtomicU32>) {
        self.shared_file_id = Some(allocator);
    }

    /// Attach a shared ModuleId allocator. Must be the same handle held
    /// by the `ModuleRegistry` and the compiler `Session` — that is
    /// what keeps the three subsystems' ModuleId values in lockstep.
    pub fn set_module_id_allocator(&mut self, allocator: Shared<AtomicU32>) {
        self.shared_module_id = Some(allocator);
    }

    /// Raw allocator handles for callers that want to construct
    /// downstream components (secondary loaders, parallel workers).
    pub fn file_id_allocator(&self) -> Option<Shared<AtomicU32>> {
        self.shared_file_id.clone()
    }

    pub fn module_id_allocator(&self) -> Option<Shared<AtomicU32>> {
        self.shared_module_id.clone()
    }

    /// Set the cog name for this loader's root. When a module path
    /// starts with this segment (e.g. `core.mesh.xds`), it's treated
    /// as rooted at the loader's `root_path` (so the file lookup
    /// becomes `mesh/xds.vr`, not `core/mesh/xds.vr`). This is what
    /// keeps the absolute form declared in source and the relative
    /// form derived from the filesystem from being treated as two
    /// distinct modules.
    pub fn set_cog_name(&mut self, cog_name: impl Into<String>) {
        self.cog_name = Some(cog_name.into());
    }

    /// Builder-style equivalent of `set_cog_name`.
    pub fn with_cog_name(mut self, cog_name: impl Into<String>) -> Self {
        self.cog_name = Some(cog_name.into());
        self
    }

    /// Current cog name, if set.
    pub fn cog_name(&self) -> Option<&str> {
        self.cog_name.as_deref()
    }

    /// Canonicalise a dotted path against the current cog name. If
    /// the path's first segment matches `cog_name`, it's stripped.
    /// Used both as the key for the loader's own caches and by
    /// `module_path_to_file_path` below.
    fn canonical_key(&self, path: &str) -> String {
        if let Some(cog) = &self.cog_name {
            if path == cog.as_str() {
                return String::new();
            }
            if let Some(rest) = path.strip_prefix(cog.as_str()) {
                if let Some(tail) = rest.strip_prefix('.') {
                    return tail.to_string();
                }
            }
        }
        path.to_string()
    }

    /// Attach a CogResolver for cross-cog module resolution.
    ///
    /// When set, `mount http.client.Response` will check if "http" is an
    /// external cog and load from its installed root path.
    pub fn with_cog_resolver(mut self, resolver: crate::cog_resolver::CogResolver) -> Self {
        self.cog_resolver = Some(resolver);
        self
    }

    /// Set the cog resolver after construction.
    pub fn set_cog_resolver(&mut self, resolver: crate::cog_resolver::CogResolver) {
        self.cog_resolver = Some(resolver);
    }

    /// Create a module loader for a specific target platform.
    ///
    /// # Arguments
    ///
    /// * `root_path` - Root directory for module search
    /// * `target_triple` - Target triple (e.g., "x86_64-unknown-linux-gnu")
    pub fn for_target(root_path: impl Into<PathBuf>, target_triple: &str) -> Self {
        Self {
            root_path: root_path.into(),
            cog_name: None,
            next_file_id: 0,
            shared_file_id: None,
            next_module_id: 0,
            shared_module_id: None,
            loaded_files: Map::new(),
            module_path_to_id: Map::new(),
            module_info_cache: Map::new(),
            cfg_evaluator: CfgEvaluator::for_target(target_triple),
            cog_resolver: None,
        }
    }

    /// Create a module loader with a custom target configuration.
    ///
    /// # Arguments
    ///
    /// * `root_path` - Root directory for module search
    /// * `config` - Target configuration for cfg evaluation
    pub fn with_config(root_path: impl Into<PathBuf>, config: TargetConfig) -> Self {
        Self {
            root_path: root_path.into(),
            cog_name: None,
            next_file_id: 0,
            shared_file_id: None,
            next_module_id: 0,
            shared_module_id: None,
            loaded_files: Map::new(),
            module_path_to_id: Map::new(),
            module_info_cache: Map::new(),
            cfg_evaluator: CfgEvaluator::with_config(config),
            cog_resolver: None,
        }
    }

    /// Get a reference to the cfg evaluator.
    pub fn cfg_evaluator(&self) -> &CfgEvaluator {
        &self.cfg_evaluator
    }

    /// Get mutable access to the cfg evaluator.
    ///
    /// Use this to enable features or customize cfg options.
    pub fn cfg_evaluator_mut(&mut self) -> &mut CfgEvaluator {
        &mut self.cfg_evaluator
    }

    /// Enable a feature flag for conditional compilation.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut loader = ModuleLoader::new("src/");
    /// loader.enable_feature("simd");
    /// loader.enable_feature("experimental");
    /// ```
    pub fn enable_feature(&mut self, feature: impl Into<Text>) {
        self.cfg_evaluator.config_mut().enable_feature(feature);
    }

    /// Load a module from a file.
    ///
    /// Searches for the module file using the mapping rules:
    /// 1. `path/to/module.vr`
    /// 2. `path/to/module/mod.vr`
    pub fn load_module(
        &mut self,
        module_path: &ModulePath,
        _module_id: ModuleId,
    ) -> ModuleResult<ModuleSource> {
        // Cross-cog resolution: if first segment matches an external cog,
        // search in the cog's root directory instead of the local root.
        // Clone the resolved root to avoid borrow conflict with &mut self.
        let cross_cog_target: Option<(PathBuf, ModulePath)> = if let Some(ref cog_resolver) = self.cog_resolver {
            let segments = module_path.segments();
            if !segments.is_empty() {
                let first_segment = segments[0].as_str();
                if cog_resolver.is_external_cog(first_segment) {
                    cog_resolver.get_cog_root(first_segment).map(|cog_root| {
                        let rest_segments: List<Text> = segments.iter().skip(1).cloned().collect();
                        let cog_module_path = if rest_segments.is_empty() {
                            ModulePath::root()
                        } else {
                            ModulePath::new(rest_segments)
                        };
                        (cog_root.clone(), cog_module_path)
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some((cog_root, cog_module_path)) = cross_cog_target {
            return self.load_module_from_root(&cog_root, &cog_module_path);
        }

        let candidates = self.find_module_file(module_path)?;
        Self::resolve_unique_candidate(module_path, &candidates)
            .and_then(|p| self.load_file(&p))
    }

    /// Load a module from a specific root directory (used for cross-cog resolution).
    fn load_module_from_root(
        &mut self,
        root: &std::path::Path,
        module_path: &ModulePath,
    ) -> ModuleResult<ModuleSource> {
        let candidates = self.find_module_file_in_root(root, module_path)?;
        Self::resolve_unique_candidate(module_path, &candidates)
            .and_then(|p| self.load_file(&p))
    }

    /// Walk every candidate path and return the unique one that exists,
    /// or surface `E_MODULE_PATH_COLLISION` when more than one exists.
    ///
    /// Concrete failure mode this prevents: project ships BOTH
    /// `src/foo.vr` (Rule 2 — file form) AND `src/foo/mod.vr` (Rule 4 —
    /// directory form). Without this check the loader silently picks
    /// the first-found candidate; the user sees `unbound variable`
    /// errors at use-sites pointing into the loser, with no hint that
    /// two declarations of the same module exist.
    fn resolve_unique_candidate(
        module_path: &ModulePath,
        candidates: &List<PathBuf>,
    ) -> ModuleResult<PathBuf> {
        let existing: Vec<PathBuf> = candidates
            .iter()
            .filter(|p| p.exists())
            .cloned()
            .collect();
        match existing.len() {
            0 => Err(ModuleError::module_not_found(
                module_path.clone(),
                candidates.clone(),
            )),
            1 => Ok(existing.into_iter().next().unwrap()),
            _ => {
                let mut iter = existing.into_iter();
                let winning_path = iter.next().unwrap();
                let losing_paths: List<PathBuf> = iter.collect();
                Err(ModuleError::PathCollision {
                    path: module_path.clone(),
                    winning_path,
                    losing_paths,
                    span: None,
                })
            }
        }
    }

    /// Find module file candidates in a specific root directory.
    fn find_module_file_in_root(&self, root: &std::path::Path, module_path: &ModulePath) -> ModuleResult<List<PathBuf>> {
        let mut candidates = List::new();

        if module_path.is_root() {
            for ext in Self::EXTENSIONS {
                candidates.push(root.join(format!("lib.{}", ext)));
            }
            for ext in Self::EXTENSIONS {
                candidates.push(root.join(format!("main.{}", ext)));
            }
        } else {
            let relative_path = self.module_path_to_file_path(module_path);

            for ext in Self::EXTENSIONS {
                let mut file_path = root.join(&relative_path);
                file_path.set_extension(ext);
                candidates.push(file_path);
            }
            for ext in Self::EXTENSIONS {
                let dir_path = root.join(&relative_path).join(format!("mod.{}", ext));
                candidates.push(dir_path);
            }
        }

        // Filter out any candidates that escape the root directory (path traversal protection)
        Self::filter_safe_paths(&candidates, root)
    }

    /// Filter candidate paths to ensure none escape the given root directory.
    /// This prevents path traversal attacks via crafted module names.
    fn filter_safe_paths(candidates: &List<PathBuf>, root: &std::path::Path) -> ModuleResult<List<PathBuf>> {
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mut safe = List::new();
        for candidate in candidates.iter() {
            // If the file exists, canonicalize and check prefix
            if let Ok(canonical) = candidate.canonicalize() {
                if canonical.starts_with(&canonical_root) {
                    safe.push(candidate.clone());
                }
                // else: path escapes root, silently skip
            } else {
                // File doesn't exist yet; check the parent directory or the logical path
                // Use lexical normalization: ensure no ".." components remain after join
                let normalized = candidate.components().collect::<PathBuf>();
                let norm_str = normalized.to_string_lossy();
                if !norm_str.contains("..") {
                    safe.push(candidate.clone());
                }
            }
        }
        Ok(safe)
    }

    /// Supported file extension.
    const EXTENSIONS: &'static [&'static str] = &["vr"];

    /// Find possible file locations for a module.
    fn find_module_file(&self, module_path: &ModulePath) -> ModuleResult<List<PathBuf>> {
        let mut candidates = List::new();

        if module_path.is_root() {
            // Root module: lib.vr or main.vr (Rule 1: crate root)
            for ext in Self::EXTENSIONS {
                candidates.push(self.root_path.join(format!("lib.{}", ext)));
            }
            for ext in Self::EXTENSIONS {
                candidates.push(self.root_path.join(format!("main.{}", ext)));
            }
        } else {
            // Convert module path to file path
            let relative_path = self.module_path_to_file_path(module_path);

            // Candidate 1: module_name.vr (Rule 2: foo.vr defines module foo)
            for ext in Self::EXTENSIONS {
                let mut file_path = self.root_path.join(&relative_path);
                file_path.set_extension(ext);
                candidates.push(file_path);
            }

            // Candidate 2: module_name/mod.vr (Rule 4: directory module with child modules)
            for ext in Self::EXTENSIONS {
                let dir_path = self
                    .root_path
                    .join(&relative_path)
                    .join(format!("mod.{}", ext));
                candidates.push(dir_path);
            }
        }

        // Filter out any candidates that escape the root directory (path traversal protection)
        Self::filter_safe_paths(&candidates, &self.root_path)
    }

    /// Convert a module path to a filesystem path.
    ///
    /// Example: `std.collections.List` → `std/collections/List`.
    ///
    /// If the first segment matches the current `cog_name`, it is
    /// stripped — the loader's `root_path` is *already* the cog
    /// root. Without this, `core.mesh.xds.resources` would try to
    /// locate `core/mesh/xds/resources.vr` under a root that is
    /// itself `core/`, producing `core/core/mesh/xds/resources.vr`
    /// which doesn't exist — and a later resolver would retry with
    /// the short form under a new ModuleId, leading to the
    /// "Conflicting export" class of bugs.
    ///
    /// Rejects path segments containing ".." or absolute path
    /// components to prevent path traversal attacks.
    fn module_path_to_file_path(&self, module_path: &ModulePath) -> PathBuf {
        let segments = module_path.segments();
        let mut path = PathBuf::new();

        let mut skip_first = false;
        if let (Some(cog), Some(first)) = (&self.cog_name, segments.first()) {
            if first.as_str() == cog.as_str() {
                skip_first = true;
            }
        }

        for (idx, segment) in segments.iter().enumerate() {
            if skip_first && idx == 0 {
                continue;
            }
            let s = segment.as_str();
            // Reject path traversal attempts and absolute path segments
            if s == ".." || s == "." || s.contains('/') || s.contains('\\') || s.contains('\0') {
                // Return an empty path that won't resolve to any file
                return PathBuf::new();
            }
            path.push(s);
        }

        path
    }

    /// Load a file and allocate a FileId.
    fn load_file(&mut self, file_path: &Path) -> ModuleResult<ModuleSource> {
        // Check cache
        if let Some(&file_id) = self.loaded_files.get(&file_path.to_path_buf()) {
            // Already loaded - read again (could cache the content too)
            let source = std::fs::read_to_string(file_path).map_err(|e| ModuleError::IoError {
                path: file_path.to_path_buf(),
                error: Text::from(e.to_string()),
                span: None,
            })?;

            return Ok(ModuleSource::new(
                file_path.to_path_buf(),
                Text::from(source),
                file_id,
            ));
        }

        // Load file
        let source = std::fs::read_to_string(file_path).map_err(|e| ModuleError::IoError {
            path: file_path.to_path_buf(),
            error: Text::from(e.to_string()),
            span: None,
        })?;

        // Allocate FileId — prefer shared counter when attached.
        let file_id = if let Some(allocator) = &self.shared_file_id {
            FileId::new(allocator.fetch_add(1, Ordering::Relaxed))
        } else {
            let id = FileId::new(self.next_file_id);
            self.next_file_id += 1;
            id
        };
        self.loaded_files.insert(file_path.to_path_buf(), file_id);

        Ok(ModuleSource::new(
            file_path.to_path_buf(),
            Text::from(source),
            file_id,
        ))
    }

    /// Parse a module source into an AST.
    ///
    /// Uses verum_parser to parse the source code into an AST module.
    /// Automatically:
    /// 1. Injects prelude import unless @![no_implicit_prelude] is present
    /// 2. Filters items based on @cfg attributes for conditional compilation
    ///
    /// Parses a module source into an AST, injecting the standard prelude
    /// import (`use std.prelude.*`) unless @![no_implicit_prelude] is present,
    /// and filtering items based on @cfg attributes for conditional compilation.
    pub fn parse_module(
        &self,
        source: &ModuleSource,
        module_id: ModuleId,
        module_path: ModulePath,
    ) -> ModuleResult<ModuleInfo> {
        // Create lexer and parser
        let lexer = Lexer::new(source.source.as_str(), source.file_id);
        let parser = VerumParser::new();

        // Parse the module source using verum_parser
        let mut ast = match parser.parse_module(lexer, source.file_id) {
            Ok(module) => module,
            Err(errors) => {
                // Collect parse errors into a single error message
                let error_messages: List<String> = errors.iter().map(|e| e.to_string()).collect();
                let combined_error = error_messages.join("; ");
                return Err(ModuleError::ParseError {
                    path: module_path,
                    error: combined_error,
                    span: None,
                });
            }
        };

        // Filter items based on @cfg attributes for conditional compilation
        // This removes items that don't match the target configuration
        self.filter_cfg_items(&mut ast);

        // Inject prelude import unless @![no_implicit_prelude] is present.
        // The standard prelude (std.prelude.*) provides Maybe, Result, List,
        // Text, Iterator, Clone, Eq, Ord, etc. It has the lowest resolution
        // priority so explicit imports can shadow prelude items.
        self.inject_prelude(&mut ast)?;

        // VUVA #145 / MOD-MED-1 — validate top-level `module foo;` /
        // `module foo { … }` declarations against the filesystem.
        // Warnings are non-blocking: the pipeline drains them and
        // emits diagnostics so the user sees dangling forward-decls
        // and inline-vs-filesystem overlaps without breaking the
        // build.
        let header_warnings =
            validate_module_headers_against_filesystem(&source.file_path, &ast);

        let mut info = ModuleInfo::new(
            module_id,
            module_path,
            ast,
            source.file_id,
            source.source.clone(),
        );
        info.header_warnings = header_warnings;
        Ok(info)
    }

    /// Filter module items based on @cfg attributes.
    ///
    /// Removes items whose @cfg predicates evaluate to false for the current
    /// target configuration. This implements conditional compilation.
    ///
    /// # Example
    ///
    /// ```verum
    /// @cfg(unix)
    /// fn unix_only() { ... }  // Removed when compiling for Windows
    ///
    /// @cfg(feature = "simd")
    /// fn simd_impl() { ... }  // Removed unless "simd" feature is enabled
    /// ```
    fn filter_cfg_items(&self, module: &mut AstModule) {
        module.items = self.cfg_evaluator.filter_items(&module.items);
    }

    /// Check if a module should be loaded based on its @cfg attributes.
    ///
    /// This is used to skip loading entire modules that don't match the
    /// target configuration (e.g., a module with @cfg(unix) when compiling
    /// for Windows).
    ///
    /// # Arguments
    ///
    /// * `attrs` - The @cfg attributes on the module declaration
    ///
    /// # Returns
    ///
    /// `true` if the module should be loaded, `false` to skip it.
    pub fn should_load_module(&self, attrs: &[verum_ast::Attribute]) -> bool {
        self.cfg_evaluator.should_include(attrs)
    }

    /// Inject the standard prelude import into a module.
    ///
    /// The prelude is automatically imported into every module unless the module
    /// has the @![no_implicit_prelude] attribute. The prelude import is added
    /// at the beginning of the import list to ensure it has the lowest resolution
    /// priority (explicit imports can shadow prelude items).
    ///
    /// Injects the standard prelude import. The prelude is inserted at the
    /// beginning of the import list so it has the lowest resolution priority
    /// (explicit imports and local bindings take precedence over prelude items).
    fn inject_prelude(&self, module: &mut AstModule) -> ModuleResult<()> {
        // Check for @![no_implicit_prelude] attribute
        if module.has_no_implicit_prelude() {
            return Ok(());
        }

        // Create prelude import: use std.prelude.*;
        // The import is created with a synthetic span at position 0
        let prelude_import = self.create_prelude_import(module.file_id);

        // Insert at the beginning to give it lowest priority in name resolution
        module.items.insert(0, prelude_import);

        Ok(())
    }

    /// Create the synthetic prelude import item.
    ///
    /// Creates: `use std.prelude.*;`
    fn create_prelude_import(&self, file_id: verum_ast::FileId) -> verum_ast::Item {
        use verum_ast::span::Span;
        use verum_ast::{
            Ident, MountDecl, MountTree, MountTreeKind, Item, ItemKind, Path, PathSegment,
        };

        // Create the path: std.prelude
        let span = Span::new(0, 0, file_id); // Synthetic span

        let mut segments = List::new();
        segments.push(PathSegment::Name(Ident::new(Text::from("std"), span)));
        segments.push(PathSegment::Name(Ident::new(Text::from("prelude"), span)));

        let path = Path::new(segments, span);

        // Create glob import tree
        let mount_tree = MountTree {
            kind: MountTreeKind::Glob(path.clone()),
            alias: Maybe::None,
            span,
        };

        // Create mount declaration
        let mount_decl = MountDecl {
            visibility: verum_ast::decl::Visibility::Private,
            tree: mount_tree,
            alias: Maybe::None,
            span,
        };

        // Wrap in Item
        Item::new(ItemKind::Mount(mount_decl), span)
    }

    /// Load and parse a module in one step.
    pub fn load_and_parse(
        &mut self,
        module_path: &ModulePath,
        module_id: ModuleId,
    ) -> ModuleResult<ModuleInfo> {
        let source = self.load_module(module_path, module_id)?;
        self.parse_module(&source, module_id, module_path.clone())
    }

    /// Get the root path.
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    /// Check if a file has been loaded.
    pub fn is_loaded(&self, file_path: &Path) -> bool {
        self.loaded_files.contains_key(&file_path.to_path_buf())
    }

    /// Clear all caches — file contents, path→id, and parsed ModuleInfo.
    /// Called between independent compilation sessions (REPL re-runs,
    /// VCS harness between test files) to avoid leaking state across
    /// sessions.
    pub fn clear_cache(&mut self) {
        self.loaded_files.clear();
        self.module_path_to_id.clear();
        self.module_info_cache.clear();
    }
}

// ============================================================================
// Lazy Module Resolution Trait
// ============================================================================

/// Trait for on-demand module resolution.
///
/// This trait enables lazy loading of modules during type checking.
/// When TypeChecker encounters an import for a module not in the registry,
/// it calls this resolver to load and register the module.
///
/// # Architecture
///
/// ```text
/// TypeChecker                         LazyModuleResolver
/// ┌─────────────────┐                ┌────────────────────────────────┐
/// │ import foo.bar  │ ─────────────> │ 1. Convert path to file path   │
/// │                 │                │ 2. Load and parse source       │
/// │ module not in   │                │ 3. Register in ModuleRegistry  │
/// │ registry        │ <───────────── │ 4. Return ModuleInfo           │
/// │                 │                └────────────────────────────────┘
/// │ retry import    │
/// └─────────────────┘
/// ```
///
/// # Usage
///
/// ```ignore
/// use verum_modules::{ModuleLoader, LazyModuleResolver, ModuleRegistry};
/// use std::sync::{Arc, Mutex};
///
/// let loader = Arc::new(Mutex::new(ModuleLoader::new("src/")));
/// let registry = ModuleRegistry::new();
///
/// // TypeChecker uses the resolver for lazy loading
/// type_checker.set_module_resolver(loader);
/// ```
///
/// Enables on-demand module resolution during type checking. When the type
/// checker encounters an import for a module not in the registry, it calls
/// the resolver to load and register it. The resolver searches for files
/// using the file system mapping rules (foo.vr or foo/mod.vr).
pub trait LazyModuleResolver: Send + Sync {
    /// Resolve a module by path, loading it if necessary.
    ///
    /// # Arguments
    ///
    /// * `module_path` - The module path (e.g., "std.collections.list")
    ///
    /// # Returns
    ///
    /// * `Ok(ModuleInfo)` - The loaded module
    /// * `Err(ModuleError)` - If the module cannot be found or parsed
    fn resolve_module(&mut self, module_path: &str) -> ModuleResult<ModuleInfo>;

    /// Check if this resolver can handle a given module path.
    ///
    /// This allows multiple resolvers to be chained (e.g., stdlib resolver,
    /// local project resolver, dependency resolver).
    ///
    /// # Arguments
    ///
    /// * `module_path` - The module path to check
    ///
    /// # Returns
    ///
    /// * `true` if this resolver can potentially handle the path
    /// * `false` if another resolver should be tried
    fn can_resolve(&self, module_path: &str) -> bool;

    /// Get the root path for this resolver (if applicable).
    fn root_path(&self) -> Option<&Path>;
}

impl LazyModuleResolver for ModuleLoader {
    fn resolve_module(&mut self, module_path: &str) -> ModuleResult<ModuleInfo> {
        // Canonical-path dedupe — see field docs on `module_path_to_id`.
        // The key strips the cog-name prefix so that `core.foo.bar` and
        // `foo.bar` (at cog_name="core") share the same cache slot.
        let key = self.canonical_key(module_path);
        if let Some(cached) = self.module_info_cache.get(&key) {
            return Ok(cached.clone());
        }
        let path = ModulePath::from_str(module_path);
        let id = if let Some(&existing) = self.module_path_to_id.get(&key) {
            existing
        } else {
            let fresh = self.allocate_module_id();
            self.module_path_to_id.insert(key.clone(), fresh);
            fresh
        };
        let info = self.load_and_parse(&path, id)?;
        self.module_info_cache.insert(key, info.clone());
        Ok(info)
    }

    fn can_resolve(&self, module_path: &str) -> bool {
        // ModuleLoader can potentially resolve any path within its root
        // The actual existence check happens in resolve_module
        let path = ModulePath::from_str(module_path);
        match self.find_module_file(&path) {
            Ok(candidates) => candidates.iter().any(|c| c.exists()),
            Err(_) => false,
        }
    }

    fn root_path(&self) -> Option<&Path> {
        Some(&self.root_path)
    }
}

impl ModuleLoader {
    /// Allocate a new ModuleId. Prefers the shared allocator when
    /// attached (the norm during pipeline compilation); falls back to
    /// the loader-local counter otherwise (standalone / test use).
    fn allocate_module_id(&mut self) -> ModuleId {
        if let Some(allocator) = &self.shared_module_id {
            ModuleId::new(allocator.fetch_add(1, Ordering::Relaxed))
        } else {
            let id = self.next_module_id;
            self.next_module_id += 1;
            ModuleId::new(id)
        }
    }
}

/// Type alias for a shared, thread-safe lazy module resolver.
pub type SharedModuleResolver = std::sync::Arc<std::sync::Mutex<dyn LazyModuleResolver>>;

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::cfg::TargetConfig;
    use verum_ast::{Attribute, Span};

    fn dummy_span() -> Span {
        Span::dummy()
    }

    #[test]
    fn test_loader_default_cfg() {
        let loader = ModuleLoader::new("/tmp");
        let config = loader.cfg_evaluator().config();

        // Host platform should be detected
        assert!(!config.target_os.is_empty());
        assert!(!config.target_arch.is_empty());
    }

    #[test]
    fn test_loader_for_target() {
        let loader = ModuleLoader::for_target("/tmp", "x86_64-unknown-linux-gnu");
        let config = loader.cfg_evaluator().config();

        assert_eq!(config.target_os.as_str(), "linux");
        assert_eq!(config.target_arch.as_str(), "x86_64");
        assert_eq!(config.target_family.as_str(), "unix");
    }

    #[test]
    fn test_loader_with_custom_config() {
        let mut config = TargetConfig::windows_x86_64();
        config.enable_feature("simd");

        let loader = ModuleLoader::with_config("/tmp", config);
        let cfg = loader.cfg_evaluator().config();

        assert_eq!(cfg.target_os.as_str(), "windows");
        assert!(cfg.has_feature("simd"));
    }

    #[test]
    fn test_enable_feature() {
        let mut loader = ModuleLoader::new("/tmp");
        loader.enable_feature("experimental");

        assert!(loader.cfg_evaluator().config().has_feature("experimental"));
    }

    #[test]
    fn test_should_load_module_no_cfg() {
        let loader = ModuleLoader::new("/tmp");

        // Empty attributes should allow loading
        let attrs: Vec<Attribute> = vec![];
        assert!(loader.should_load_module(&attrs));
    }

    fn make_ident_path(name: &str, span: Span) -> verum_ast::ty::Path {
        use verum_ast::{Ident, ty::{Path, PathSegment}};
        Path::new(
            List::from(vec![PathSegment::Name(Ident::new(Text::from(name), span))]),
            span,
        )
    }

    #[test]
    fn test_should_load_module_matching_cfg() {
        let loader = ModuleLoader::for_target("/tmp", "x86_64-unknown-linux-gnu");

        // Create @cfg(unix) attribute
        let cfg_expr = verum_ast::Expr::new(
            verum_ast::expr::ExprKind::Path(make_ident_path("unix", dummy_span())),
            dummy_span(),
        );
        let attr = Attribute::new(
            Text::from("cfg"),
            Maybe::Some(List::from(vec![cfg_expr])),
            dummy_span(),
        );

        // On Linux, @cfg(unix) should pass
        assert!(loader.should_load_module(&[attr]));
    }

    #[test]
    fn test_should_load_module_non_matching_cfg() {
        let loader = ModuleLoader::for_target("/tmp", "x86_64-unknown-linux-gnu");

        // Create @cfg(windows) attribute
        let cfg_expr = verum_ast::Expr::new(
            verum_ast::expr::ExprKind::Path(make_ident_path("windows", dummy_span())),
            dummy_span(),
        );
        let attr = Attribute::new(
            Text::from("cfg"),
            Maybe::Some(List::from(vec![cfg_expr])),
            dummy_span(),
        );

        // On Linux, @cfg(windows) should fail
        assert!(!loader.should_load_module(&[attr]));
    }

    #[test]
    fn test_should_load_module_feature_check() {
        use verum_ast::literal::StringLit;

        let mut loader = ModuleLoader::new("/tmp");
        loader.enable_feature("simd");

        // Create @cfg(feature = "simd") attribute
        let key_expr = verum_ast::Expr::new(
            verum_ast::expr::ExprKind::Path(make_ident_path("feature", dummy_span())),
            dummy_span(),
        );
        let value_expr = verum_ast::Expr::new(
            verum_ast::expr::ExprKind::Literal(verum_ast::literal::Literal {
                kind: verum_ast::literal::LiteralKind::Text(StringLit::Regular(Text::from("simd"))),
                span: dummy_span(),
            }),
            dummy_span(),
        );
        let cfg_expr = verum_ast::Expr::new(
            verum_ast::expr::ExprKind::Binary {
                left: Box::new(key_expr),
                op: verum_ast::expr::BinOp::Assign,
                right: Box::new(value_expr),
            },
            dummy_span(),
        );
        let attr = Attribute::new(
            Text::from("cfg"),
            Maybe::Some(List::from(vec![cfg_expr])),
            dummy_span(),
        );

        // With "simd" feature enabled, should pass
        assert!(loader.should_load_module(&[attr.clone()]));

        // Without "simd" feature, should fail
        let loader_no_simd = ModuleLoader::new("/tmp");
        assert!(!loader_no_simd.should_load_module(&[attr]));
    }

    #[test]
    fn test_cfg_evaluator_access() {
        let mut loader = ModuleLoader::new("/tmp");

        // Test mutable access
        loader.cfg_evaluator_mut().config_mut().set_custom("my_cfg", "enabled");

        // Verify custom cfg is set
        assert!(loader.cfg_evaluator().config().is_set("my_cfg"));
        assert!(loader.cfg_evaluator().config().matches("my_cfg", "enabled"));
    }
}
