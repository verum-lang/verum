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
async fn architecture_projection_preserves_explicit_arguments_owner_and_evidence_revision() {
    let project = Project::new();
    let source = "// 😀 source identity\n@arch_module(\n    foundation: Foundation.ZfcTwoInacc,\n    stratum: MsfsStratum.LFnd,\n    lifecycle: Lifecycle.Definition\n)\nmodule example.main;\nfn main() -> Int { 42 }";
    let uri = project.open("main.vr", source);
    let snapshot = project.snapshot(&uri).await;
    assert_eq!(snapshot["parsed"], true, "{snapshot:#}");
    assert_eq!(snapshot["architecture"], json!([]), "item attributes must not move to the AST root");
    let module = snapshot["declarations"].as_array().unwrap().iter().find(|item| item["kind"] == "module").unwrap();
    let annotation = &module["architecture"][0];
    assert_eq!(annotation["state"], "declared");
    assert_eq!(annotation["canonical"]["foundation"], "ZfcTwoInacc");
    assert_eq!(annotation["canonical"]["stratum"], "LFnd");
    assert_eq!(annotation["canonical"]["lifecycle"], "Definition");
    assert_eq!(annotation["supplied"].as_array().unwrap().len(), 3);
    assert_eq!(annotation["supplied"][0]["name"], "foundation");
    assert_eq!(annotation["supplied"][0]["value"]["source"], "Foundation.ZfcTwoInacc");
    assert_eq!(annotation["supplied"][0]["value"]["range"]["start"], json!({"line":2,"character":16}));
    assert_eq!(annotation["attribute"]["range"]["start"], json!({"line":1,"character":0}));
    assert!(annotation["canonical"].get("at_tier").is_some());
    assert!(annotation["supplied"].as_array().unwrap().iter().all(|argument| argument["name"] != "at_tier"), "canonical defaults must not become explicit arguments");
    assert!(snapshot["declarations"].as_array().unwrap().iter().filter(|item| item["kind"] == "function").all(|item| item["architecture"] == json!([])));
    let params = crate::project::EvidenceParams {
        snapshot: SnapshotParams { text_document: TextDocumentIdentifier { uri: uri.clone() }, version: 1 },
        expected_source_hash: snapshot["source_hash"].as_str().unwrap().into(),
        expected_project_revision: snapshot["project_revision"].as_str().map(str::to_owned),
        declaration_id: Some(module["id"].as_str().unwrap().into()),
    };
    let evidence = project.service.inner().handle_document_evidence(params).await.unwrap();
    assert_eq!(evidence["declaration"]["architecture"], module["architecture"]);
    let changed = source.replace("Lifecycle.Definition", "Lifecycle.Theorem");
    project.service.inner().documents.try_update(&uri, changed, 2).unwrap();
    let next = project.service.inner().handle_document_snapshot(SnapshotParams {
        text_document: TextDocumentIdentifier { uri }, version: 2,
    }).await.unwrap();
    assert_ne!(next["source_hash"], snapshot["source_hash"]);
    let next_module = next["declarations"].as_array().unwrap().iter().find(|item| item["kind"] == "module").unwrap();
    assert_eq!(next_module["architecture"][0]["state"], "declared", "a lifecycle marker is not verification evidence");
    assert!(next_module["architecture"][0].get("verified").is_none());
}

#[tokio::test]
async fn architecture_projection_keeps_invalid_unrelated_and_unparsed_states_distinct() {
    let project = Project::new();
    let uri = project.open("invalid.vr", "@arch_module(stratum: MsfsStratum.DoesNotExist)\nmodule invalid;");
    let invalid = project.snapshot(&uri).await;
    assert_eq!(invalid["parsed"], true, "{invalid:#}");
    let annotation = &invalid["declarations"][0]["architecture"][0];
    assert_eq!(annotation["state"], "invalid");
    assert!(annotation["message"].as_str().unwrap().contains("DoesNotExist"));
    assert!(annotation.get("canonical").is_none());
    assert_eq!(annotation["supplied"][0]["value"]["source"], "MsfsStratum.DoesNotExist");
    let unrelated = project.open("unrelated.vr", "@deprecated(\"arch_module foundation: Foundation.ZfcTwoInacc\")\nfn ordinary() -> Int { 1 }");
    let unrelated = project.snapshot(&unrelated).await;
    assert_eq!(unrelated["parsed"], true, "{unrelated:#}");
    assert_eq!(unrelated["declarations"][0]["architecture"], json!([]));
    let broken = project.open("broken.vr", "@arch_module(stratum:");
    let broken = project.snapshot(&broken).await;
    assert_eq!(broken["parsed"], false);
    assert_eq!(broken["architecture"], Value::Null, "failed parsing is not an empty declaration set");
}

#[tokio::test]
async fn explicit_script_language_changes_project_semantics_without_changing_source() {
    let project = Project::new();
    let source = "public fn value() -> Int { 42 } let mut i = 0; while i < 3 { i = i + 1; }";
    let uri = project.open("entry.vr", source);
    let before = project.snapshot(&uri).await;
    assert_eq!(before["parsed"], false);
    project.service.inner().did_close(DidCloseTextDocumentParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
    }).await;
    project.service.inner().did_open(DidOpenTextDocumentParams {
        text_document: TextDocumentItem {
            uri: uri.clone(), language_id: "verum-script".into(),
            version: 1, text: source.into(),
        },
    }).await;
    let script = project.snapshot(&uri).await;
    assert_eq!(script["parsed"], true, "{script:#}");
    assert!(errors(&script).is_empty(), "{script:#}");
    assert_ne!(before["project_revision"], script["project_revision"]);
    let consumer = project.open("consumer.vr", "mount entry.value; fn f() -> Int { value() }");
    let imported = project.snapshot(&consumer).await;
    assert!(!errors(&imported).is_empty(), "an entry's explicit script mode cannot authorize importing its unmarked statements: {imported:#}");
    project.service.inner().did_close(DidCloseTextDocumentParams {
        text_document: TextDocumentIdentifier { uri: consumer.clone() },
    }).await;
    std::fs::remove_file(consumer.to_file_path().unwrap()).unwrap();
    project.service.inner().did_close(DidCloseTextDocumentParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
    }).await;
    project.service.inner().did_open(DidOpenTextDocumentParams {
        text_document: TextDocumentItem {
            uri: uri.clone(), language_id: "verum".into(),
            version: 1, text: source.into(),
        },
    }).await;
    let restored = project.snapshot(&uri).await;
    assert_eq!(restored["parsed"], false);
    assert_eq!(restored["project_revision"], before["project_revision"]);
}

#[tokio::test]
async fn project_mount_failure_carries_its_source_range_and_clears_after_repair() {
    let project = Project::new();
    let dependency = project.file("dependency.vr", DEP);
    let uri = project.open("main.vr", "mount dependency.missing;");
    let before = project.snapshot(&uri).await;
    let mount = &before["mounts"][0];
    assert_eq!(mount["resolution"]["state"], "unresolved", "{before:#}");
    assert!(mount["resolution"]["reason"].as_str().is_some_and(|s| !s.is_empty()));
    assert_eq!(mount["range"]["start"]["line"], 0);
    std::fs::write(dependency.to_file_path().unwrap(), "public fn missing() -> Int { 1 }").unwrap();
    let after = project.snapshot(&uri).await;
    assert_eq!(after["mounts"][0]["resolution"]["state"], "resolved", "{after:#}");
    assert_ne!(before["project_revision"], after["project_revision"]);
}

#[tokio::test]
async fn project_topology_resolves_consumers_in_closed_sources() {
    let project = Project::new();
    let dep = project.open("dependency.vr", DEP);
    let consumer = project.file("consumer.vr", MAIN);
    let broken = project.file("broken.vr", "@cfg(feature = \"disabled_in_test\") mount dependency.*;\nmount dependency.missing;");
    let _ = project.snapshot(&dep).await;
    let snapshot = project.service.inner().handle_project_snapshot(Value::Null).await.unwrap();
    let graph = &snapshot["import_topology"];
    assert_eq!(graph["inspected_modules"], 3, "{snapshot:#}");
    assert_eq!(graph["unresolved_mounts"], 1);
    let failures = graph["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0]["location"]["uri"], broken.to_string());
    assert_eq!(failures[0]["location"]["range"]["start"]["line"], 1);
    assert_eq!(failures[0]["source"], "mount dependency.missing;");
    assert!(failures[0]["reason"].as_str().unwrap().contains("missing"));
    assert_eq!(graph["edges"].as_array().unwrap().len(), 1);
    let edge = &graph["edges"][0];
    assert_eq!(edge["location"]["uri"], consumer.to_string());
    assert_eq!(edge["names"], json!(["Token", "answer"]));
    assert_eq!(snapshot["documents"].as_array().unwrap().len(), 1);
}

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
    assert_eq!(snapshot["mounts"][0]["resolution"], json!({"state":"resolved","bindings":{"start":0,"end":2}}));
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
                source_declaration: None,
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
    metadata.version = verum_types::core_metadata::CORE_METADATA_VERSION;
    let library = crate::library_metadata::LibraryMetadata::decode(
        &verum_types::core_metadata::encode_metadata(&metadata).unwrap(),
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
#[ignore = "requires matching VERUM_TEST_LIBRARY_METADATA from a fresh stdlib bake"]
async fn actual_stdlib_nominal_universes_reach_project_analysis() {
    let path = std::env::var_os("VERUM_TEST_LIBRARY_METADATA").expect("fresh metadata path");
    let library = crate::library_metadata::LibraryMetadata::load(std::path::Path::new(&path)).unwrap();
    let project = Project::with_library(Some(library.clone()));
    let uri = project.open("main.vr", "mount core.math.hott.{NType}; fn keep(x: &NType<0>) -> &NType<0> { x }");
    let snapshot = project.snapshot(&uri).await;
    assert!(errors(&snapshot).is_empty(), "{snapshot:#}");
    assert_eq!(snapshot["analysis_scope"], "project-sources-with-library-metadata");
    assert_eq!(snapshot["library_metadata"]["sha256"], library.identity.sha256.as_str());
    assert!(snapshot["bindings"].as_array().unwrap().iter().any(|binding|
        binding["source_module"] == "core.math.hott" && binding["name"] == "NType"
    ), "{snapshot:#}");
    for (body, diagnostic) in [
        ("fn change(x: &NType<0>) -> &NType<1> { x }", "mismatch"),
        ("fn read(x: &NType<0>) -> Type { x.carrier }", "private"),
    ] {
        let rejected = Project::with_library(Some(library.clone()));
        let uri = rejected.open("bad.vr", &format!("mount core.math.hott.{{NType}}; {body}"));
        let snapshot = rejected.snapshot(&uri).await;
        assert!(errors(&snapshot).iter().any(|d| d["message"].as_str()
            .is_some_and(|m| m.to_lowercase().contains(diagnostic))), "{snapshot:#}");
    }
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

fn nominal_library_fixture(target: &str) -> std::sync::Arc<crate::library_metadata::LibraryMetadata> {
    use verum_types::core_metadata::*;
    let mut metadata = CoreMetadata { version: CORE_METADATA_VERSION, ..Default::default() };
    for (owner, source) in [
        ("portable.levels", "public type Level is { public value: Type(1) };"),
        ("foreign.levels", "public type Level is { public value: Type(1) };"),
        ("portable.api", "public type Carrier is { public value: Type(1), public nested: PrivateLevel };"),
    ] {
        let ast = verum_parser::Parser::new(source).parse_module().unwrap();
        let verum_ast::ItemKind::Type(decl) = &ast.items[0].kind else { panic!("type") };
        let verum_ast::TypeDeclBody::Record(fields) = &decl.body else { panic!("record") };
        let key: verum_common::Text = format!("{owner}.{}", decl.name.name).into();
        metadata.type_declaration_order.push(key.clone());
        metadata.types.insert(key, TypeDescriptor {
            source_declaration: verum_ast::source_signature::source_type_declaration(decl).unwrap(),
            name: decl.name.name.clone(), module_path: owner.into(), origin_module_path: None,
            generic_params: Default::default(),
            kind: TypeDescriptorKind::Record { fields: fields.iter().map(|field| FieldDescriptor {
                name: field.name.name.clone(), ty: "__opaque_src".into(), is_public: true,
                declared_visibility: Some(verum_ast::Visibility::Public),
            }).collect() },
            size: None, alignment: None, methods: Default::default(), implements: Default::default(),
            decl_span: None, is_public: true, is_transparent_wrapper: false,
            resource_discipline: Default::default(),
        });
    }
    let mut bindings = verum_common::OrderedMap::new();
    bindings.insert("PrivateLevel".into(), Some(target.into()));
    metadata.declaration_type_bindings.insert("portable.api".into(), bindings);
    crate::library_metadata::LibraryMetadata::decode(&encode_metadata(&metadata).unwrap()).unwrap()
}

#[tokio::test]
async fn nominal_library_levels_and_private_scopes_reach_project_analysis() {
    let library = nominal_library_fixture("portable.levels.Level");
    let project = Project::with_library(Some(library.clone()));
    let uri = project.open("main.vr", "mount portable.api.{Carrier}; fn read(x: &Carrier) -> Type(1) { x.value } fn nested(x: &Carrier) -> portable.levels.Level { x.nested }");
    let before = project.snapshot(&uri).await;
    assert!(errors(&before).is_empty(), "{before:#}");
    assert_eq!(before["library_metadata"]["sha256"], library.identity.sha256.as_str());
    let negative = Project::with_library(Some(library));
    let bad = negative.open("bad.vr", "mount portable.api.{Carrier}; fn bad(x: &Carrier) -> Type(0) { x.value }");
    assert!(!errors(&negative.snapshot(&bad).await).is_empty(), "LSP erased a universe level");

    let changed = nominal_library_fixture("foreign.levels.Level");
    let mut context = crate::project_context::ProjectContext::with_library(Some(changed));
    context.configure(project.root.path().to_path_buf(), None);
    *project.service.inner().project.lock() = context;
    let after = project.snapshot(&uri).await;
    assert_ne!(before["project_revision"], after["project_revision"]);
    assert!(!errors(&after).is_empty(), "changed private binding reused stale successful analysis: {after:#}");
    assert!(errors(&after).iter().any(|diagnostic| diagnostic["message"].as_str().is_some_and(|message|
        message.contains("portable.levels.Level") && message.contains("foreign.levels.Level"))),
        "expected the changed nominal owner mismatch: {after:#}");
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

#[tokio::test]
async fn unsaved_project_entries_share_dependency_types_and_retire_on_close() {
    for (relative, owner) in [
        ("draft.vr", "demo.draft"),
        ("research/draft.vr", "demo.research.draft"),
        ("src/draft.vr", "demo.draft"),
    ] {
        let project = Project::new();
        project.file("Verum.toml", "[cog]\nname='demo'\n");
        let dependency = project.file("src/dependency.vr", DEP);
        let path = project.root.path().join(relative);
        let uri = Url::from_file_path(&path).unwrap();
        let source = "mount demo.dependency.{Token, answer}; public fn read(value: Token) -> Int { value.id + answer() }";
        let backend = project.service.inner();
        backend.documents.open(uri.clone(), source.into(), 1);
        assert!(!path.exists(), "the entry must remain unsaved");
        let first = project.snapshot(&uri).await;
        assert!(first["parsed"].as_bool().unwrap());
        assert!(errors(&first).is_empty(), "{relative}: {first:#}");
        let token = first["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|binding| binding["name"] == "Token")
            .unwrap();
        assert_eq!(token["definition"]["uri"], dependency.as_str());
        assert!(!token["type_display"].as_str().unwrap().is_empty());
        let consumer = project.open("src/consumer.vr", &format!("mount {owner}.read;"));
        assert!(errors(&project.snapshot(&consumer).await).is_empty());
        backend
            .documents
            .try_update(&uri, source.replace("value.id + answer()", "true"), 2)
            .unwrap();
        let invalid = backend
            .handle_document_snapshot(SnapshotParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                version: 2,
            })
            .await
            .unwrap();
        assert!(
            !errors(&invalid).is_empty(),
            "invalid return type must be diagnosed: {invalid:#}"
        );
        assert!(
            errors(&invalid).iter().all(|error| !error["message"]
                .as_str()
                .unwrap()
                .contains("unavailable in this source generation")),
            "{invalid:#}"
        );
        assert_ne!(first["project_revision"], invalid["project_revision"]);
        backend.documents.close(&uri);
        let closed = project.snapshot(&consumer).await;
        assert!(
            !errors(&closed).is_empty(),
            "closed unsaved entry cannot leave stale exports: {closed:#}"
        );
        let graph = backend.handle_project_snapshot(Value::Null).await.unwrap();
        assert!(
            graph["sources"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["uri"] != uri.as_str())
        );
        assert!(!path.exists());
    }
}

#[tokio::test]
async fn root_entry_uses_buffer_bytes_but_cannot_shadow_a_source_module() {
    let project = Project::new();
    project.file("Verum.toml", "[cog]\nname='demo'\n");
    project.file("src/dependency.vr", DEP);
    let uri = project.file("draft.vr", "invalid disk contents");
    let source = "mount demo.dependency.answer; public fn read() -> Int { answer() }";
    project
        .service
        .inner()
        .documents
        .open(uri.clone(), source.into(), 1);
    assert!(errors(&project.snapshot(&uri).await).is_empty());
    let collision = project.file("src/draft.vr", "public fn other() -> Bool { true }");
    let ambiguous = project.snapshot(&uri).await;
    assert!(!errors(&ambiguous).is_empty(), "{ambiguous:#}");
    let graph = project
        .service
        .inner()
        .handle_project_snapshot(Value::Null)
        .await
        .unwrap();
    assert!(
        graph["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue.as_str().unwrap().contains("E_MODULE_PATH_COLLISION")),
        "{graph:#}"
    );
    std::fs::remove_file(collision.to_file_path().unwrap()).unwrap();
    assert!(errors(&project.snapshot(&uri).await).is_empty());
}

#[tokio::test]
async fn unsaved_script_entry_obeys_the_same_source_ownership_collisions() {
    let project = Project::new();
    project.file("Verum.toml", "[cog]\nname='demo'\n");
    project.file("src/dependency.vr", DEP);
    let uri = Url::from_file_path(project.root.path().join("draft.vr")).unwrap();
    project.service.inner().documents.open_with_source_mode(
        uri.clone(),
        "mount demo.dependency.answer; let value = answer();".into(),
        1,
        verum_common::source_mode::SourceMode::Script,
    );
    let valid = project.snapshot(&uri).await;
    assert!(errors(&valid).is_empty(), "{valid:#}");
    let collision = project.file("src/draft.vr", "public fn other() -> Bool { true }");
    let invalid = project.snapshot(&uri).await;
    assert!(!errors(&invalid).is_empty(), "script parsing cannot grant owner override: {invalid:#}");
    std::fs::remove_file(collision.to_file_path().unwrap()).unwrap();
    assert!(errors(&project.snapshot(&uri).await).is_empty());
}

#[tokio::test]
async fn script_entry_cannot_replace_an_installed_library_owner() {
    let project = Project::with_library(Some(library_fixture("Int")));
    project.file("Verum.toml", "[cog]\nname='portable'\n");
    let consumer = project.open("src/main.vr", "mount portable.units.Count; fn read(value: Count) -> Int { value }");
    let uri = Url::from_file_path(project.root.path().join("units.vr")).unwrap();
    let source = "let value = 42; print(value);";
    assert!(crate::source_parsing::parse(source, verum_ast::FileId::new(0)).is_err(),
        "this control must exercise the script-only checker path");
    project.service.inner().documents.open_with_source_mode(
        uri.clone(), source.into(), 1,
        verum_common::source_mode::SourceMode::Script,
    );
    let refused = project.snapshot(&uri).await;
    assert!(!errors(&refused).is_empty(), "an entry must not be checked using a library's source identity: {refused:#}");
    let imported = project.snapshot(&consumer).await;
    assert!(errors(&imported).is_empty(), "library ownership must survive an invalid entry: {imported:#}");
    assert_eq!(imported["bindings"][0]["type_display"], "Int");
}

#[tokio::test]
async fn project_root_and_main_lib_modules_do_not_alias_each_others_source() {
    let project = Project::new();
    let root = project.open("mod.vr", "public fn root_value() -> Int { 1 }");
    let main = project.open("main.vr", "fn main() -> Int { 2 }");
    let lib = project.open("lib.vr", "public fn library_value() -> Int { 3 }");
    for uri in [&root, &main, &lib] {
        let snapshot = project.snapshot(uri).await;
        assert!(errors(&snapshot).is_empty(), "{snapshot:#}");
    }
    let graph = project.service.inner().handle_project_snapshot(Value::Null).await.unwrap();
    for (uri, owner) in [(root, "<root>"), (main, "main"), (lib, "lib")] {
        let source = graph["sources"].as_array().unwrap().iter()
            .find(|source| source["uri"] == uri.as_str()).unwrap();
        assert_eq!(source["module"], owner);
    }
    assert!(graph["issues"].as_array().unwrap().is_empty(), "{graph:#}");
}

#[tokio::test]
async fn local_and_external_modules_keep_distinct_owners_and_binding_types() {
    let project = Project::new();
    project.file(
        "Verum.toml",
        "[cog]\nname='demo'\n[dependencies]\nmath={path='vendor/math'}\n",
    );
    project.file("vendor/math/Verum.toml", "[cog]\nname='math'\n");
    let external = project.file("vendor/math/src/item.vr", "public fn value() -> Bool { true }");
    let local = project.file("src/math/item.vr", "public fn value() -> Int { 42 }");
    let entry = project.open("src/main.vr", "mount math.item.value as external; mount demo.math.item.value as local; fn first() -> Bool { external() } fn second() -> Int { local() }");
    let result = project.snapshot(&entry).await;
    assert!(errors(&result).is_empty(), "{result:#}");
    for (name, uri, expected) in [("external", external, "Bool"), ("local", local, "Int")] {
        let binding = result["bindings"].as_array().unwrap().iter()
            .find(|binding| binding["name"] == name).unwrap();
        assert_eq!(binding["definition"]["uri"], uri.as_str());
        assert!(binding["type_display"].as_str().unwrap().contains(expected), "{binding:#}");
    }
}

#[tokio::test]
async fn source_library_owner_conflict_refuses_the_generation_and_recovers_on_close() {
    let project = Project::with_library(Some(library_fixture("Int")));
    project.file("Verum.toml", "[cog]\nname='portable'\n");
    let consumer = project.open("src/main.vr", "mount portable.units.Count; fn read(value: Count) -> Int { value }");
    let before = project.snapshot(&consumer).await;
    assert!(errors(&before).is_empty(), "{before:#}");
    assert_eq!(before["bindings"][0]["type_display"], "Int");
    let uri = Url::from_file_path(project.root.path().join("units.vr")).unwrap();
    let source = "let value = 42;";
    assert!(crate::source_parsing::parse(source, verum_ast::FileId::new(0)).is_ok(),
        "this control must exercise importable source declarations");
    project.service.inner().documents.open_with_source_mode(
        uri.clone(), source.into(), 1, verum_common::source_mode::SourceMode::Script,
    );
    let refused = project.snapshot(&uri).await;
    assert!(!errors(&refused).is_empty(), "{refused:#}");
    let conflicted = project.snapshot(&consumer).await;
    assert!(errors(&conflicted).iter().any(|diagnostic| diagnostic["message"].as_str().unwrap().contains("conflicts with an installed library module")), "{conflicted:#}");
    assert!(conflicted["project_revision"].is_null());
    assert!(conflicted["bindings"].as_array().unwrap().is_empty());
    project.service.inner().documents.close(&uri);
    let recovered = project.snapshot(&consumer).await;
    assert!(errors(&recovered).is_empty(), "{recovered:#}");
    assert_eq!(recovered["bindings"], before["bindings"]);
    assert_eq!(recovered["library_metadata"], before["library_metadata"]);
    assert_eq!(recovered["project_revision"], before["project_revision"]);
}
