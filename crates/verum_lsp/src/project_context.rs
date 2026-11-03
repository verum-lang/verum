//! Project source generations shared by all language-service clients.
use crate::{document::DocumentStore, position_utils::ast_span_to_range};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{Location, Url};
use verum_ast::{FileId, ItemKind, Module};
use verum_common::{List, Map, Set, Text};
use verum_common::source_mode::SourceMode;
use verum_modules::{
    ExportTable, ImportResolver, ModuleId, ModuleInfo, ModuleLoader, ModulePath, ModuleRegistry,
    SharedModuleRegistry, extract_exports_from_module, resolve_glob_reexports,
    resolve_specific_reexport_kinds,
};
use verum_types::TypeChecker;

#[derive(Clone, Debug, Serialize)]
pub struct ProjectBinding {
    pub name: Text,
    pub type_display: Text,
    pub source_module: Text,
    pub definition: Option<Location>,
}
/// Outcome of resolving this specific mount in the document's project generation.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MountResolution {
    Resolved { bindings: std::ops::Range<usize> },
    Unresolved { reason: Text },
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceIdentity {
    pub source_mode: SourceMode,
    pub module: Text,
    pub uri: Url,
    pub source_hash: Text,
    pub version: Option<i32>,
    pub parsed: bool,
}
#[derive(Clone)]
struct Source {
    source_mode: SourceMode,
    path: PathBuf,
    text: Text,
    file_id: FileId,
    ast: Option<Module>,
    // An editor's explicit entry policy must not change how peers import it.
    module_ast: Option<Module>,
    version: Option<i32>,
    owner: ModulePath,
    module_id: Option<ModuleId>,
}
impl Source {
    fn dependency_ast(&self) -> Option<&Module> {
        if self.source_mode == SourceMode::Script { self.module_ast.as_ref() }
        else { self.ast.as_ref() }
    }
}
struct AnalysisReceipt {
    file_id: FileId,
    revision: Text,
    reads: verum_modules::ModuleObservations,
}
#[derive(Default, Serialize)]
pub struct ImportTopology {
    inspected_modules: usize,
    unresolved_mounts: usize,
    failures: Vec<ImportFailure>,
    edges: Vec<ImportEdge>,
}

#[derive(Clone, Serialize)]
struct ImportSite {
    importer: Text,
    location: Location,
    source: Text,
}

#[derive(Serialize)]
struct ImportFailure {
    #[serde(flatten)]
    site: ImportSite,
    reason: Text,
}

#[derive(Serialize)]
struct ImportEdge {
    #[serde(flatten)]
    site: ImportSite,
    provider: Text,
    names: Vec<Text>,
}

#[derive(Default)]
pub struct ProjectContext {
    control: Option<crate::AnalysisControl>,
    library: Option<std::sync::Arc<crate::library_metadata::LibraryMetadata>>,
    root: Option<PathBuf>,
    library_roots: Map<Text, PathBuf>,
    revision: Text,
    configuration: Text,
    analyses: Map<PathBuf, AnalysisReceipt>,
    sources: Map<PathBuf, Source>,
    pub identities: List<SourceIdentity>,
    exports: Map<ModuleId, ExportTable>,
    module_paths: Map<ModuleId, ModulePath>,
    cfg: verum_ast::cfg::CfgEvaluator,
    topology: Option<ImportTopology>,
    pub issues: List<Text>,
}
impl ProjectContext {
    pub fn with_library(
        library: Option<std::sync::Arc<crate::library_metadata::LibraryMetadata>>,
    ) -> Self {
        Self {
            library,
            ..Self::default()
        }
    }
    pub fn set_analysis_control(&mut self, control: Option<crate::AnalysisControl>) {
        self.control = control;
    }
    pub fn library_identity(&self) -> Option<&crate::library_metadata::LibraryIdentity> {
        self.library.as_ref().map(|library| &library.identity)
    }
    pub fn configure(&mut self, root: PathBuf, options: Option<&serde_json::Value>) {
        self.root = Some(root);
        self.library_roots.clear();
        self.revision = Text::new();
        self.topology = None;
        self.exports.clear();
        self.module_paths.clear();
        self.analyses.clear();
        if let Some(roots) = options
            .and_then(|o| o.get("libraryRoots"))
            .and_then(|v| v.as_object())
        {
            self.library_roots = roots
                .iter()
                .filter_map(|(name, value)| {
                    Some((name.clone().into(), PathBuf::from(value.as_str()?)))
                })
                .collect();
        }
    }
    pub fn invalidate(&mut self, reason: &str) {
        self.revision = Text::new();
        self.topology = None;
        self.exports.clear();
        self.module_paths.clear();
        self.analyses.clear();
        self.identities.clear();
        self.issues = List::from_iter([Text::from(reason)]);
    }
    /// Resolve the indexed sources on the first graph request for this generation.
    /// Inspecting closed files does not open them or type-check their bodies.
    pub fn import_topology(&mut self) -> Result<&ImportTopology> {
        if self.topology.is_none() {
            let control = self.analysis_control();
            let mut graph = ImportTopology::default();
            for source in self.sources.values() {
                control.check()?;
                let (Some(ast), Some(id)) = (&source.ast, source.module_id) else { continue };
                if !self.module_paths.contains_key(&id) { continue; }
                let uri = Url::from_file_path(&source.path).map_err(|_| anyhow::anyhow!("Source URI"))?;
                graph.inspected_modules += 1;
                let mut resolver = ImportResolver::new();
                for item in &ast.items {
                    let ItemKind::Mount(decl) = &item.kind else { continue };
                    if !self.cfg.should_include_item(item) { continue; }
                    control.check()?;
                    let span = item.span.start as usize..item.span.end as usize;
                    let site = ImportSite {
                        importer: source.owner.to_string().into(),
                        location: Location::new(uri.clone(), ast_span_to_range(&item.span, &source.text)),
                        source: source.text.get(span).context("Import span outside captured source")?.into(),
                    };
                    let import = match resolver.resolve_import(decl, id, &self.exports, &self.module_paths) {
                        Ok(import) => import,
                        Err(error) => {
                            graph.failures.push(ImportFailure { site, reason: error.to_string().into() });
                            continue;
                        }
                    };
                    let mut providers = std::collections::BTreeMap::<Text, Vec<Text>>::new();
                    for binding in import.items {
                        let provider = self.module_paths.get(&binding.source_module)
                            .context("Resolved binding has no registered module")?;
                        providers.entry(provider.to_string().into()).or_default().push(binding.name);
                    }
                    for (provider, mut names) in providers {
                        names.sort();
                        graph.edges.push(ImportEdge { site: site.clone(), provider, names });
                    }
                }
            }
            let site_order = |a: &ImportSite, b: &ImportSite| a.importer.cmp(&b.importer)
                .then_with(|| a.location.range.start.cmp(&b.location.range.start));
            graph.edges.sort_by(|a, b| site_order(&a.site, &b.site).then_with(|| a.provider.cmp(&b.provider)));
            graph.failures.sort_by(|a, b| site_order(&a.site, &b.site));
            graph.unresolved_mounts = graph.failures.len();
            control.check()?;
            self.topology = Some(graph);
        }
        Ok(self.topology.as_ref().expect("topology set after complete traversal"))
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    fn analysis_control(&self) -> crate::AnalysisControl {
        self.control.clone().unwrap_or_else(|| {
            crate::AnalysisControl::with_deadline(std::time::Instant::now() + crate::ANALYSIS_TIMEOUT)
        })
    }
    pub fn refresh(&mut self, documents: &DocumentStore) -> Result<bool> {
        let control = self.analysis_control();
        control.check()?;
        let Some(root) = self.root.clone() else {
            return Ok(false);
        };
        let mut roots: List<(Text, PathBuf)> = List::new();
        let mut cog = Text::new();
        let mut configuration = Text::new();
        let mut has_manifest = false;
        for name in ["Verum.toml", "verum.toml"] {
            let manifest = root.join(name);
            if manifest.is_file() {
                has_manifest = true;
                configuration = std::fs::read_to_string(&manifest)?.into();
                let value: toml::Value =
                    toml::from_str(&configuration).context("Project manifest")?;
                cog = value
                    .get("cog")
                    .and_then(|c| c.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .replace('-', "_")
                    .into();
                if let Some(dependencies) = value.get("dependencies").and_then(|d| d.as_table()) {
                    for (name, dependency) in dependencies {
                        if let Some(path) = dependency.get("path").and_then(|p| p.as_str()) {
                            let location = verum_modules::cog_resolver::CogLocation {
                                name: name.clone().into(),
                                version: Text::new(),
                                root_path: root.join(path).canonicalize()?,
                                artifact_kind: verum_modules::cog_resolver::CogArtifactKind::Source,
                            };
                            roots.push((name.clone().into(), location.source_root()));
                        }
                    }
                }
                break;
            }
        }
        // A manifest selects the project layout. In a loose workspace, `src`
        // is an ordinary namespace and must not hide files beside it.
        let source_root = if has_manifest && root.join("src").is_dir() {
            root.join("src")
        } else {
            root.clone()
        };
        roots.push((Text::new(), source_root.clone()));
        for (name, path) in &self.library_roots {
            roots.push((name.clone(), path.canonicalize()?));
        }
        let mut captured = Map::new();
        let mut owners = Map::new();
        let mut bytes = 0;
        for (prefix, directory) in &roots {
            capture(
                directory,
                directory,
                prefix,
                &mut captured,
                &mut owners,
                &mut bytes,
                &control,
            )?;
        }
        // Project compilation and editor analysis share the same source identity.
        for path in captured.keys() {
            if path.starts_with(&source_root) {
                owners.insert(
                    path.clone(),
                    ModulePath::for_source_file(&source_root, &cog, path),
                );
            }
        }
        let mut open = documents.flat_collect_from_documents(|uri, doc| {
            uri.to_file_path()
                .ok()
                .map(|path| {
                    vec![(
                        path,
                        doc.text.clone(),
                        doc.version,
                        doc.file_id,
                        doc.module.clone(),
                        doc.source_mode,
                    )]
                })
                .unwrap_or_default()
        });
        open.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, text, ..) in &open {
            if !owners.contains_key(path) {
                if path.starts_with(&source_root) {
                    owners.insert(
                        path.clone(),
                        ModulePath::for_source_file(&source_root, &cog, path),
                    );
                } else if path.starts_with(&root) {
                    owners.insert(path.clone(), ModulePath::for_source_file(&root, &cog, path));
                } else {
                    continue;
                }
            }
            captured.insert(path.clone(), Text::from(text.as_str()));
        }
        ensure!(captured.len() <= 4096, "Project source limit exceeded");
        let mut total = 0usize;
        for text in captured.values() {
            ensure!(text.len() <= 512 * 1024, "Source exceeds analysis limit");
            total += text.len();
        }
        ensure!(total <= 64 * 1024 * 1024, "Project byte limit exceeded");
        let mut paths: List<_> = captured.keys().cloned().collect();
        paths.sort();
        let mut hash = Sha256::new();
        if let Some(library) = &self.library {
            hash.update(library.identity.sha256.as_bytes());
        }
        hash.update([0]);
        hash.update(configuration.as_bytes());
        roots.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        for (prefix, path) in &roots {
            hash.update(format!("{prefix}:{path:?}\0").as_bytes());
        }
        let configuration: Text = hex::encode(hash.clone().finalize()).into();
        if configuration != self.configuration {
            self.analyses.clear();
        }
        for path in &paths {
            hash.update(path.to_string_lossy().as_bytes());
            hash.update([0]);
            let mode = open.iter().find(|(p, ..)| p == path).map_or(SourceMode::Source, |entry| entry.5);
            hash.update(mode.as_str().as_bytes());
            hash.update([0]);
            hash.update(captured.get(path).unwrap().as_bytes());
            hash.update([0]);
        }
        // A semantic generation depends on effective sources, not which editor
        // tabs happen to be open. Document versions and incarnations remain
        // independently guarded by DocumentStore when publishing results.
        let revision: Text = hex::encode(hash.finalize()).into();
        let pending = open.iter().any(|(path, ..)| {
            captured.contains_key(path)
                && Url::from_file_path(path).ok().is_some_and(|uri| {
                    documents.with_document(&uri, |doc| {
                        doc.analysis_revision.as_deref() != Some(revision.as_str())
                    }).unwrap_or(false)
                })
        });
        if revision == self.revision && !pending {
            // Open/close and version-only changes still update source receipts.
            for identity in &mut self.identities {
                identity.version = identity.uri.to_file_path().ok().and_then(|path| {
                    open.iter().find(|(p, ..)| p == &path).map(|(_, _, version, ..)| *version)
                });
            }
            return Ok(false);
        }
        let mut sources = Map::new();
        for path in paths {
            control.check()?;
            let text = captured.remove(&path).unwrap();
            let opened = open.iter().find(|(p, ..)| p == &path);
            let source_mode = opened.map_or(SourceMode::Source, |entry| entry.5);
            // Compiler source identity is independent of an editor incarnation.
            // Reopening identical text must not rewrite every imported AST span.
            let (file_id, ast, version) = if let Some(old) = self
                .sources
                .get(&path)
                .filter(|old| old.text == text)
            {
                let ast = if old.source_mode == source_mode { old.ast.clone() }
                    else { crate::source_parsing::parse_with_mode(&text, old.file_id, source_mode).ok() };
                (old.file_id, ast, opened.map(|(_, _, version, ..)| *version))
            } else if let Some((_, _, version, id, ast, _)) = opened {
                (*id, ast.clone(), Some(*version))
            } else {
                let uri = Url::from_file_path(&path).map_err(|_| anyhow::anyhow!("Source URI"))?;
                let id = documents.get_file_id(&uri);
                let ast = crate::source_parsing::parse(&text, id).ok();
                (id, ast, None)
            };
            let module_ast = if source_mode == SourceMode::Script {
                self.sources.get(&path)
                    .filter(|old| old.text == text)
                    .map(|old| old.dependency_ast().cloned())
                    .unwrap_or_else(|| crate::source_parsing::parse(&text, file_id).ok())
            } else { None };
            sources.insert(
                path.clone(),
                Source {
                    source_mode,
                    path: path.clone(),
                    text,
                    file_id,
                    ast,
                    module_ast,
                    version,
                    owner: owners.remove(&path).unwrap(),
                    module_id: self.sources.get(&path).and_then(|old| old.module_id),
                },
            );
        }
        let mut loader = ModuleLoader::new(&source_root);
        if !cog.is_empty() {
            loader.set_cog_name(cog.as_str());
        }
        let snapshot: Map<_, _> = sources
            .iter()
            .map(|(path, source)| (path.clone(), (source.owner.clone(), source.text.clone())))
            .collect();
        loader.set_owned_source_snapshot(snapshot);
        let mut cogs = verum_modules::cog_resolver::CogResolver::new();
        for (prefix, directory) in &roots {
            if !prefix.is_empty() {
                cogs.register_cog(prefix.clone(), "", directory.clone());
            }
        }
        loader.set_cog_resolver(cogs);
        let mut registry = self
            .library
            .as_ref()
            .map(|library| library.registry.deep_clone())
            .unwrap_or_else(ModuleRegistry::new);
        registry.set_cog_name(cog.as_str());
        for (prefix, _) in &roots {
            if !prefix.is_empty() {
                registry.register_external_cog(prefix.clone());
            }
        }
        // Retain source identities across generations so adding a module does
        // not renumber unrelated declarations and their export provenance.
        for source in sources.values() {
            if let Some(id) = source.module_id { registry.reserve_id(id); }
        }
        let mut source_paths: List<_> = sources.keys().cloned().collect();
        source_paths.sort();
        for path in source_paths {
            let source = sources.get_mut(&path).unwrap();
            source.module_id.get_or_insert_with(|| registry.allocate_id());
        }
        let mut module_sources = Map::new();
        let mut issues = List::new();
        let mut ordered: List<_> = sources.values().collect();
        ordered.sort_by(|a, b| a.path.cmp(&b.path));
        for source in ordered {
            control.check()?;
            let Some(ast) = source.dependency_ast() else {
                issues.push(format!("Cannot parse {}", source.path.display()).into());
                continue;
            };
            // All cogs use the compiler's candidate and collision rules.
            if let Err(error) = loader.load_module(&source.owner, source.module_id.unwrap()) {
                issues.push(error.to_string().into());
                continue;
            }
            let id = source.module_id.unwrap();
            ensure!(
                registry
                    .get_by_path(source.owner.to_string().as_str())
                    .is_none(),
                "Source module {} conflicts with an installed library module",
                source.owner
            );
            let mut ast = ast.clone();
            ast.items = loader.cfg_evaluator().filter_items(&ast.items);
            let mut info = ModuleInfo::new(
                id,
                source.owner.clone(),
                ast.clone(),
                source.file_id,
                source.text.clone(),
            );
            match extract_exports_from_module(&ast, id, &source.owner) {
                Ok(exports) => info.exports = exports,
                Err(error) => issues.push(error.to_string().into()),
            }
            registry.register(info);
            module_sources.insert(id, source.clone());
        }
        if let Err(error) = resolve_glob_reexports(&mut registry) {
            issues.push(error.to_string().into());
        }
        if let Err(error) = resolve_specific_reexport_kinds(&mut registry) {
            issues.push(error.to_string().into());
        }
        let exports = registry
            .all_modules()
            .map(|(id, info)| (*id, info.exports.clone()))
            .collect();
        let module_paths = registry
            .all_modules()
            .map(|(id, info)| (*id, info.path.clone()))
            .collect();
        let base = std::sync::Arc::new(registry);
        for (path, _, _, document_file_id, _, _) in &open {
            control.check()?;
            let Some(source) = sources.get(path) else {
                continue;
            };
            let uri = Url::from_file_path(path).map_err(|_| anyhow::anyhow!("Source URI"))?;
            if documents.with_document(&uri, |doc| {
                doc.analysis_revision.as_deref() == Some(revision.as_str())
            }).unwrap_or(false) {
                continue;
            }
            if let Some(receipt) = self.analyses.get_mut(path) {
                let reusable = documents.with_document(&uri, |doc| {
                    doc.file_id == receipt.file_id
                        && doc.analysis_revision.as_deref() == Some(receipt.revision.as_str())
                }).unwrap_or(false) && receipt.reads.rebase_if_unchanged(base.clone());
                if reusable {
                    documents.with_document_mut(&uri, |doc| {
                        doc.analysis_revision = Some(revision.clone());
                    });
                    receipt.revision = revision.clone();
                    continue;
                }
            }
            let (observed, reads) = base.observe_reads();
            let registry = SharedModuleRegistry::from_registry(observed);
            if source.source_mode == SourceMode::Script && source.module_ast.is_none() {
                if let Some(ast) = &source.ast {
                    let id = source.module_id.expect("source identity allocated");
                    match loader.load_module(&source.owner, id) {
                        Err(error) => issues.push(error.to_string().into()),
                        Ok(_) if registry.read().get_by_path(source.owner.to_string().as_str()).is_some() => {
                            issues.push(format!("Source module {} conflicts with an installed library module", source.owner).into());
                        }
                        Ok(_) => {
                            let mut ast = ast.clone();
                            ast.items = loader.cfg_evaluator().filter_items(&ast.items);
                            let mut info = ModuleInfo::new(id, source.owner.clone(), ast.clone(), source.file_id, source.text.clone());
                            info.exports = extract_exports_from_module(&ast, id, &source.owner)?;
                            // Only this entry's checker sees the script AST. The shared
                            // dependency registry retains source-declared module rules.
                            // This local write conservatively disables read-set reuse.
                            registry.write().register(info);
                        }
                    }
                }
            }
            let info = registry.read().get_by_path(source.owner.to_string().as_str())
                .filter(|info| Some(info.id) == source.module_id);
            let Some(info) = info else {
                documents.with_document_mut(&uri, |doc| {
                    doc.analysis_revision = Some(revision.clone());
                    doc.library_metadata = self.library_identity().cloned();
                    doc.project_bindings.clear();
                    doc.mount_resolutions.clear();
                    if doc.module.is_some() {
                        doc.diagnostics =
                            List::from_iter([verum_diagnostics::DiagnosticBuilder::error()
                                .message(format!(
                                    "Module {} is unavailable in this source generation",
                                    source.owner
                                ))
                                .build()]);
                    }
                });
                continue;
            };
            let mut checker = if let Some(library) = &self.library {
                let mut checker = TypeChecker::new_with_core_snapshot(library.snapshot.clone());
                checker.set_module_registry(registry.as_inner().clone());
                checker
            } else {
                TypeChecker::with_shared_registry(registry.clone())
            };
            checker.set_analysis_control(control.clone());
            checker.seed_module_properties(&info.ast.items);
            checker.register_builtins();
            if let Some(library) = &self.library {
                checker.register_stdlib_types_for_module(&info.ast);
                for (name, declaration) in &library.metadata().context_decl_nodes {
                    control.check()?;
                    checker.register_protocol_as_context(name.clone());
                    checker.register_stdlib_context_full(name.clone(), declaration.clone());
                }
            }
            checker.set_user_code_phase();
            checker.set_current_cog(cog.clone());
            checker.set_current_module(info.id);
            checker.set_current_module_path(info.path.to_string());
            control.check()?;
            documents.with_document_mut(&uri, |doc| -> verum_types::Result<()> {
                doc.check_project(&mut checker, &info.ast, revision.clone())?;
                doc.library_metadata = self.library_identity().cloned();
                let mut imports = ImportResolver::new();
                {
                    for item in &info.ast.items {
                        if let ItemKind::Mount(decl) = &item.kind {
                            let import = match imports.resolve_import(decl, info.id, &exports, &module_paths) {
                                Ok(import) => import,
                                Err(error) => {
                                    doc.mount_resolutions.insert(item.span.start, MountResolution::Unresolved {
                                        reason: error.to_string().into(),
                                    });
                                    continue;
                                }
                            };
                            let start = doc.project_bindings.len();
                            for binding in import.items {
                                // Navigation and type displays share the same dependency receipt.
                                registry.read().get(binding.source_module);
                                let definition = definition_location(
                                    &registry,
                                    &module_sources,
                                    &exports,
                                    &module_paths,
                                    binding.source_module,
                                    binding.original_name,
                                );
                                let ty = if binding.kind == verum_modules::ExportKind::Type {
                                    checker.resolved_type(binding.name.as_str()).cloned()
                                } else {
                                    None
                                }
                                .or_else(|| {
                                    checker
                                        .ctx_env_lookup(binding.name.as_str())
                                        .map(|scheme| scheme.ty.clone())
                                });
                                let type_display =
                                    ty.map(|ty| ty.to_string()).unwrap_or_default().into();
                                doc.project_bindings.push(ProjectBinding {
                                    name: binding.name,
                                    type_display,
                                    source_module: module_paths
                                        .get(&binding.source_module)
                                        .map(|path| path.to_string().into())
                                        .unwrap_or_default(),
                                    definition,
                                });
                            }
                            doc.mount_resolutions.insert(item.span.start, MountResolution::Resolved {
                                bindings: start..doc.project_bindings.len(),
                            });
                        }
                    }
                }
                Ok(())
            }).transpose()?;
            self.analyses.insert(path.clone(), AnalysisReceipt {
                file_id: *document_file_id, revision: revision.clone(), reads,
            });
        }
        control.check()?;
        self.identities = sources
            .values()
            .map(|source| SourceIdentity {
                source_mode: source.source_mode,
                module: source.owner.to_string().into(),
                uri: Url::from_file_path(&source.path).unwrap(),
                source_hash: hex::encode(Sha256::digest(source.text.as_bytes())).into(),
                version: source.version,
                parsed: source.ast.is_some(),
            })
            .collect();
        self.identities.sort_by(|a, b| a.uri.cmp(&b.uri));
        self.analyses.retain(|path, _| open.iter().any(|(p, ..)| p == path));
        self.configuration = configuration;
        self.sources = sources;
        self.cfg = loader.cfg_evaluator().clone();
        self.exports = exports;
        self.module_paths = module_paths;
        self.topology = None;
        self.issues = issues;
        self.revision = revision;
        Ok(true)
    }
}
fn module_path(root: &Path, path: &Path, prefix: &str) -> Result<ModulePath> {
    let relative = path.strip_prefix(root)?;
    let mut parts: List<Text> = relative
        .with_extension("")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned().into())
        .collect();
    if parts
        .last()
        .is_some_and(|p| p == "mod" || (parts.len() == 1 && (p == "main" || p == "lib")))
    {
        parts.pop();
    }
    let mut all = List::new();
    if !prefix.is_empty() {
        all.push(prefix.into());
    }
    all.extend(parts);
    Ok(ModulePath::new(all))
}
fn capture(
    root: &Path,
    directory: &Path,
    prefix: &str,
    sources: &mut Map<PathBuf, Text>,
    owners: &mut Map<PathBuf, ModulePath>,
    bytes: &mut usize,
    control: &crate::AnalysisControl,
) -> Result<()> {
    ensure!(
        directory.strip_prefix(root)?.components().count() <= 64,
        "Project directory depth exceeded"
    );
    for entry in std::fs::read_dir(directory)? {
        control.check()?;
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.')
            || matches!(name.as_ref(), "target" | "node_modules" | "build" | "dist")
        {
            continue;
        }
        if kind.is_dir() {
            capture(root, &path, prefix, sources, owners, bytes, control)?;
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "vr") {
            ensure!(sources.len() < 4096, "Project source limit exceeded");
            let size = entry.metadata()?.len();
            ensure!(
                size <= 512 * 1024,
                "Source exceeds analysis limit: {}",
                path.display()
            );
            let text: Text = std::fs::read_to_string(&path)?.into();
            *bytes += text.len();
            ensure!(*bytes <= 64 * 1024 * 1024, "Project byte limit exceeded");
            owners.insert(path.clone(), module_path(root, &path, prefix)?);
            sources.insert(path, text);
        }
    }
    Ok(())
}

/// Follow the compiler-resolved public mount graph, preserving aliases at each hop.
/// A cycle or unavailable source has no invented navigation target.
fn definition_location(
    registry: &SharedModuleRegistry,
    sources: &Map<ModuleId, Source>,
    exports: &Map<ModuleId, ExportTable>,
    paths: &Map<ModuleId, ModulePath>,
    mut module: ModuleId,
    mut name: Text,
) -> Option<Location> {
    let mut visited = Set::new();
    let mut resolver = ImportResolver::new();
    while visited.insert((module, name.clone())) {
        let info = registry.read().get(module)?;
        let source = sources.get(&module)?;
        for item in &info.ast.items {
            let declared = match &item.kind {
                ItemKind::Function(decl) => Some(&decl.name),
                ItemKind::Type(decl) => Some(&decl.name),
                ItemKind::Const(decl) => Some(&decl.name),
                ItemKind::Context(decl) => Some(&decl.name),
                ItemKind::Protocol(decl) => Some(&decl.name),
                ItemKind::Module(decl) => Some(&decl.name),
                _ => None,
            };
            if let Some(ident) = declared.filter(|ident| ident.as_str() == name.as_str()) {
                return Some(Location {
                    uri: Url::from_file_path(&source.path).ok()?,
                    range: ast_span_to_range(&ident.span, &source.text),
                });
            }
        }
        let next = info.ast.items.iter().find_map(|item| {
            let ItemKind::Mount(decl) = &item.kind else {
                return None;
            };
            if decl.visibility != verum_ast::Visibility::Public {
                return None;
            }
            resolver
                .resolve_import(decl, module, exports, paths)
                .ok()?
                .items
                .into_iter()
                .find(|binding| binding.name == name)
        })?;
        module = next.source_module;
        name = next.original_name;
    }
    None
}
