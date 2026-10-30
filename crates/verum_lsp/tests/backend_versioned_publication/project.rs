use super::*;
use crate::project::SnapshotParams;
use serde_json::{Value, json};

struct Project {
    root: tempfile::TempDir,
    service: tower_lsp::LspService<Backend>,
    _socket: tower_lsp::ClientSocket,
}
impl Project {
    fn new() -> Self {
        Self::with_library(None)
    }
    fn with_library(
        library: Option<std::sync::Arc<crate::library_metadata::LibraryMetadata>>,
    ) -> Self {
        let root = tempfile::tempdir().unwrap();
        let (service, socket) = crate::service::service_with_library(library);
        service
            .inner()
            .project
            .lock()
            .configure(root.path().to_path_buf(), None);
        service
            .inner()
            .config
            .apply_json(&json!({"lintEnabled":false}));
        Self {
            root,
            service,
            _socket: socket,
        }
    }
    fn file(&self, name: &str, source: &str) -> Url {
        let path = self.root.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, source).unwrap();
        Url::from_file_path(path).unwrap()
    }
    fn open(&self, name: &str, source: &str) -> Url {
        let uri = self.file(name, source);
        self.service
            .inner()
            .documents
            .open(uri.clone(), source.into(), 1);
        uri
    }
    async fn snapshot(&self, uri: &Url) -> Value {
        self.service
            .inner()
            .handle_document_snapshot(SnapshotParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                version: 1,
            })
            .await
            .unwrap()
    }
}
fn errors(snapshot: &Value) -> Vec<&Value> {
    snapshot["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["severity"] == 1)
        .collect()
}
const DEP: &str = "public type Token is { public id: Int }; public fn answer() -> Int { 42 }";
const MAIN: &str =
    "mount dependency.{Token, answer};\nfn read(value: Token) -> Int { value.id + answer() }";

#[tokio::test]
async fn imports_supply_real_types_completion_hover_and_definition() {
    let project = Project::new();
    let dep = project.file("dependency.vr", DEP);
    let main = project.open("main.vr", MAIN);
    let snapshot = project.snapshot(&main).await;
    assert!(errors(&snapshot).is_empty(), "{snapshot:#}");
    assert_eq!(snapshot["analysis_scope"], "project-sources-with-builtins");
    assert_eq!(snapshot["bindings"].as_array().unwrap().len(), 2);
    assert!(
        snapshot["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|binding| !binding["type_display"].as_str().unwrap().is_empty()),
        "{snapshot:#}"
    );
    let backend = project.service.inner();
    let result = backend
        .completion(
            serde_json::from_value(
                json!({"textDocument":{"uri":main},"position":{"line":1,"character":0}}),
            )
            .unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    let items = serde_json::to_value(result).unwrap();
    assert!(
        items
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["label"] == "answer" && i["detail"].as_str().unwrap().contains("Int")),
        "{items}"
    );
    let params = json!({"textDocument":{"uri":main},"position":{"line":1,"character":44}});
    let hover = backend
        .hover(serde_json::from_value(params.clone()).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(serde_json::to_string(&hover).unwrap().contains("Int"));
    let definition = backend
        .goto_definition(serde_json::from_value(params).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(definition).unwrap()["uri"],
        dep.as_str()
    );
}

#[tokio::test]
async fn dependency_edits_invalidate_unchanged_consumers_and_publication() {
    let project = Project::new();
    let dep = project.open("dependency.vr", DEP);
    let main = project.open("main.vr", MAIN);
    let before = project.snapshot(&main).await;
    assert!(errors(&before).is_empty(), "{before:#}");
    let backend = project.service.inner();
    let receipt = backend.documents.diagnostic_snapshot(&main).unwrap();
    backend
        .documents
        .try_update(&dep, DEP.replace("-> Int { 42 }", "-> Bool { true }"), 2)
        .unwrap();
    let after = project.snapshot(&main).await;
    assert!(!errors(&after).is_empty(), "{after:#}");
    assert_eq!(after["version"], 1);
    assert_ne!(before["project_revision"], after["project_revision"]);
    assert!(!backend.documents.is_current_snapshot(&main, &receipt));
    backend.documents.close(&dep);
    let reverted = project.snapshot(&main).await;
    assert!(errors(&reverted).is_empty(), "{reverted:#}");
    assert_ne!(after["project_revision"], reverted["project_revision"]);
}

#[tokio::test]
async fn broken_removed_and_private_dependencies_do_not_leave_exports() {
    let project = Project::new();
    let dep = project.file("dependency.vr", DEP);
    let main = project.open("main.vr", MAIN);
    assert!(errors(&project.snapshot(&main).await).is_empty());
    for invalid in [
        "public fn answer(",
        "type Token is { public id: Int }; fn answer() -> Int { 42 }",
    ] {
        std::fs::write(dep.to_file_path().unwrap(), invalid).unwrap();
        let result = project.snapshot(&main).await;
        assert!(!errors(&result).is_empty(), "{result:#}");
        assert!(
            result["bindings"].as_array().unwrap().is_empty(),
            "{result:#}"
        );
    }
    std::fs::remove_file(dep.to_file_path().unwrap()).unwrap();
    assert!(!errors(&project.snapshot(&main).await).is_empty());
}

#[tokio::test]
async fn aliases_reexports_and_path_dependencies_share_compiler_resolution() {
    let project = Project::new();
    project.file(
        "Verum.toml",
        "[cog]\nname='demo'\n[dependencies]\nmath={path='vendor/math'}\n",
    );
    project.file("vendor/math/Verum.toml", "[cog]\nname='math'\n");
    project.file("vendor/math/src/lib.vr", "public fn answer() -> Int { 42 }");
    let dep = project.file("src/dependency.vr", DEP);
    project.file(
        "src/facade.vr",
        "public mount demo.dependency.{Token, answer};",
    );
    let main = project.open("src/main.vr", "mount demo.facade.{Token, answer as local}; mount math.{answer}; fn read(value: Token) -> Int { value.id + local() + answer() }");
    let result = project.snapshot(&main).await;
    assert!(errors(&result).is_empty(), "{result:#}");
    let local = result["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "local")
        .unwrap();
    assert_eq!(local["definition"]["uri"], dep.as_str());
}

#[tokio::test]
async fn source_collisions_and_bad_manifests_fail_closed_and_recover() {
    let project = Project::new();
    project.file("dependency.vr", DEP);
    let main = project.open("main.vr", MAIN);
    assert!(errors(&project.snapshot(&main).await).is_empty());
    let manifest = project.file("Verum.toml", "invalid manifest !");
    let broken = project.snapshot(&main).await;
    assert!(!errors(&broken).is_empty());
    assert!(broken["project_revision"].is_null());
    std::fs::remove_file(manifest.to_file_path().unwrap()).unwrap();
    assert!(errors(&project.snapshot(&main).await).is_empty());
    project.file("dependency/mod.vr", DEP);
    let collision = project.snapshot(&main).await;
    assert!(!errors(&collision).is_empty(), "{collision:#}");
}

#[tokio::test]
async fn glob_reexports_preserve_types_and_omit_disabled_declarations() {
    let project = Project::new();
    project.file("dependency.vr", "@cfg(feature = \"unavailable\") public fn hidden() -> Missing { absent } public fn answer() -> Int { 42 }");
    project.file("facade.vr", "public mount dependency.*;");
    let main = project.open("main.vr", "mount facade.*; fn main() -> Int { answer() }");
    let snapshot = project.snapshot(&main).await;
    assert!(errors(&snapshot).is_empty(), "{snapshot:#}");
    assert_eq!(snapshot["bindings"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["bindings"][0]["name"], "answer");
}

fn library_fixture(target: &str) -> std::sync::Arc<crate::library_metadata::LibraryMetadata> {
    use verum_common::{List, Maybe, ResourceDiscipline};
    use verum_types::core_metadata::*;
    let mut metadata = CoreMetadata::default();
    for (name, is_public) in [("Count", true), ("Hidden", false)] {
        metadata.types.insert(
            name.into(),
            TypeDescriptor {
                name: name.into(),
                module_path: "portable.units".into(),
                origin_module_path: Maybe::None,
                generic_params: List::new(),
                kind: TypeDescriptorKind::Alias {
                    target: target.into(),
                },
                size: Maybe::None,
                alignment: Maybe::None,
                methods: List::new(),
                implements: List::new(),
                decl_span: Maybe::None,
                is_transparent_wrapper: false,
                resource_discipline: ResourceDiscipline::default(),
                is_public,
            },
        );
    }
    let library = crate::library_metadata::LibraryMetadata::decode(
        &bincode::serialize(&metadata).unwrap(),
    )
    .unwrap();
    let units = library.registry.get_by_path("portable.units").unwrap();
    assert!(units.exports.get(&"Count".into()).is_some());
    assert!(units.exports.get(&"Hidden".into()).is_none());
    library
}

#[tokio::test]
async fn release_metadata_supplies_import_types_without_inventing_source_locations() {
    let library = library_fixture("Int");
    let project = Project::with_library(Some(library.clone()));
    let uri = project.open(
        "main.vr",
        "mount portable.units.{Count}; fn read(value: Count) -> Int { value }",
    );
    let snapshot = project.snapshot(&uri).await;
    assert!(errors(&snapshot).is_empty(), "{snapshot:#}");
    assert_eq!(
        snapshot["analysis_scope"],
        "project-sources-with-library-metadata"
    );
    assert_eq!(
        snapshot["library_metadata"]["sha256"],
        library.identity.sha256.as_str()
    );
    assert_eq!(snapshot["bindings"][0]["type_display"], "Int");
    assert_eq!(snapshot["bindings"][0]["source_module"], "portable.units");
    assert!(snapshot["bindings"][0]["definition"].is_null());
    let private = project.open("private.vr", "mount portable.units.{Hidden}; fn main() {} ");
    assert!(!errors(&project.snapshot(&private).await).is_empty());
}

#[tokio::test]
async fn library_artifact_identity_invalidates_project_generation() {
    let project = Project::with_library(Some(library_fixture("Int")));
    let uri = project.open(
        "main.vr",
        "mount portable.units.{Count}; fn read(value: Count) -> Int { value }",
    );
    let before = project.snapshot(&uri).await;
    assert!(errors(&before).is_empty(), "{before:#}");
    let mut context =
        crate::project_context::ProjectContext::with_library(Some(library_fixture("Bool")));
    context.configure(project.root.path().to_path_buf(), None);
    *project.service.inner().project.lock() = context;
    let after = project.snapshot(&uri).await;
    assert_ne!(before["project_revision"], after["project_revision"]);
    assert_ne!(before["library_metadata"], after["library_metadata"]);
    assert!(!errors(&after).is_empty(), "{after:#}");
}

#[tokio::test]
async fn tab_lifecycle_reuses_semantics_without_reusing_publication_incarnations() {
    let project = Project::new();
    let dependency = project.file("dependency.vr", DEP);
    let main = project.open("main.vr", MAIN);
    let before = project.snapshot(&main).await;
    let backend = project.service.inner();
    let checks = |uri: &Url| backend.documents.with_document(uri, |doc| doc.project_check_count).unwrap();
    assert_eq!(checks(&main), 1);
    backend.documents.open(dependency.clone(), DEP.into(), 1);
    let opened = project.snapshot(&dependency).await;
    assert_eq!(opened["project_revision"], before["project_revision"]);
    assert_eq!(checks(&main), 1, "opening unchanged source must not recheck consumers");
    assert_eq!(checks(&dependency), 1);
    let old = backend.documents.diagnostic_snapshot(&dependency).unwrap();
    backend.documents.close(&dependency);
    let closed = project.snapshot(&main).await;
    assert_eq!(closed["project_revision"], before["project_revision"]);
    assert_eq!(checks(&main), 1);
    let inventory = backend.handle_project_snapshot(json!({})).await.unwrap();
    assert!(inventory["sources"].as_array().unwrap().iter()
        .find(|s| s["uri"] == dependency.as_str()).unwrap()["version"].is_null());
    backend.documents.open(dependency.clone(), DEP.into(), 1);
    project.snapshot(&dependency).await;
    assert!(!backend.documents.is_current_snapshot(&dependency, &old));
    assert_eq!(checks(&main), 1);
    assert_eq!(checks(&dependency), 1, "new incarnation must be checked independently");
    backend.documents.try_update(&dependency, DEP.into(), 2).unwrap();
    let unchanged = project.snapshot(&main).await;
    assert_eq!(unchanged["project_revision"], before["project_revision"]);
    assert_eq!(checks(&main), 1);
    assert_eq!(checks(&dependency), 2, "new document version needs its own result");
    backend.documents.try_update(&dependency, DEP.replace("-> Int { 42 }", "-> Bool { true }"), 3).unwrap();
    let changed = project.snapshot(&main).await;
    assert_ne!(changed["project_revision"], before["project_revision"]);
    assert!(!errors(&changed).is_empty());
    assert_eq!(checks(&main), 2, "real dependency changes invalidate consumers");
}

#[tokio::test]
async fn interrupted_project_has_no_stale_verdict_and_recovers_on_next_request() {
    let project = Project::new();
    project.file("dependency.vr", DEP);
    let main = project.open("main.vr", MAIN);
    let before = project.snapshot(&main).await;
    assert!(errors(&before).is_empty());
    let backend = project.service.inner();
    let control = crate::AnalysisControl::default();
    control.cancel();
    backend.set_analysis_control(Some(control));
    let interrupted = project.snapshot(&main).await;
    assert!(interrupted["project_revision"].is_null());
    assert!(interrupted["bindings"].as_array().unwrap().is_empty());
    assert!(interrupted["declarations"].as_array().unwrap().iter().all(|d| d["type_display"].is_null()));
    assert!(errors(&interrupted).iter().any(|d| d["message"].as_str().unwrap().contains("cancelled")));
    backend.set_analysis_control(None);
    let recovered = project.snapshot(&main).await;
    assert!(errors(&recovered).is_empty(), "{recovered:#}");
    assert_eq!(recovered["project_revision"], before["project_revision"]);
    assert_eq!(recovered["bindings"], before["bindings"]);
}

#[tokio::test]
async fn independent_edits_reuse_checked_modules_but_advance_publication_revision() {
    let project = Project::new();
    let dependency = project.file("dependency.vr", DEP);
    let main = project.open("main.vr", MAIN);
    let before = project.snapshot(&main).await;
    let backend = project.service.inner();
    let checks = || backend.documents.with_document(&main, |doc| doc.project_check_count).unwrap();
    let old = backend.documents.diagnostic_snapshot(&main).unwrap();
    // Opening a previously captured dependency gives it a new editor incarnation,
    // but must preserve the compiler identity of its unchanged source.
    backend.documents.open(dependency.clone(), DEP.into(), 1);
    project.snapshot(&dependency).await;
    // Insert before existing paths in lexical order: IDs must not be renumbered.
    let independent = project.open("aaa.vr", "fn unrelated() -> Int { 1 }");
    project.snapshot(&independent).await;
    assert_eq!(checks(), 1);
    let after = project.snapshot(&main).await;
    assert_ne!(before["project_revision"], after["project_revision"]);
    assert!(!backend.documents.is_current_snapshot(&main, &old));
    assert_eq!(before["bindings"], after["bindings"]);
    backend.documents.try_update(&independent, "fn unrelated() -> Int { true }".into(), 2).unwrap();
    assert!(errors(&project.snapshot(&main).await).is_empty());
    assert_eq!(checks(), 1);
    backend.documents.close(&dependency);
    std::fs::write(dependency.to_file_path().unwrap(), DEP.replace("-> Int { 42 }", "-> Bool { true }")).unwrap();
    assert!(!errors(&project.snapshot(&main).await).is_empty());
    assert_eq!(checks(), 2);
}

#[tokio::test]
async fn reexport_changes_and_new_previously_missing_modules_invalidate_observations() {
    let project = Project::new();
    let dependency = project.file("dependency.vr", DEP);
    project.file("facade.vr", "public mount dependency.*;");
    let main = project.open("main.vr", &MAIN.replace("dependency", "facade"));
    let initial = project.snapshot(&main).await;
    assert!(errors(&initial).is_empty(), "{initial:#}");
    std::fs::write(dependency.to_file_path().unwrap(), DEP.replace("-> Int { 42 }", "-> Bool { true }")).unwrap();
    assert!(!errors(&project.snapshot(&main).await).is_empty());
    std::fs::remove_file(dependency.to_file_path().unwrap()).unwrap();
    assert!(!errors(&project.snapshot(&main).await).is_empty());
    project.file("dependency.vr", DEP);
    assert!(errors(&project.snapshot(&main).await).is_empty());
}
