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
        let root = tempfile::tempdir().unwrap();
        let (service, socket) = crate::service::service();
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
