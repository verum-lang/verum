use serde_json::{Value, json};
use verum_workbench::{Config, Studio};

fn studio(root: &std::path::Path) -> Studio {
    Studio::open(Config {
        workspace: root.to_path_buf(),
        verum: "verum".into(),
        proof: "verum-source-replay".into(),
        codex: "codex".into(),
        port: 0,
    })
    .unwrap()
}

#[tokio::test]
async fn loose_workspace_keeps_root_files_and_nested_module_owners() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("helper.vr"),
        "public fn answer() -> Int { 42 }").unwrap();
    std::fs::write(root.path().join("src/helper.vr"),
        "public fn answer() -> Bool { true }").unwrap();
    let studio = studio(root.path());
    for (module, result_type, valid) in [
        ("helper", "Int", true),
        ("src.helper", "Bool", true),
        ("src.helper", "Int", false),
    ] {
        let source = format!("mount {module}.{{answer}}; fn main() -> {result_type} {{ answer() }}");
        let analysis = studio.call("language.request", json!({
            "path":"lesson.vr", "source":source,
        })).await.unwrap();
        assert_eq!(analysis["result"]["parsed"], true, "{analysis}");
        let diagnostics = analysis["result"]["diagnostics"].as_array().unwrap();
        if valid {
            assert!(diagnostics.is_empty(), "{analysis}");
        } else {
            assert!(diagnostics.iter().any(|d| {
                let message = d["message"].as_str().unwrap_or("");
                message.contains("Int") && message.contains("Bool")
            }), "a genuine type mismatch is required: {analysis}");
        }
    }
}

#[tokio::test]
async fn script_analysis_uses_compiler_entry_scope_and_original_diagnostic_lines() {
    let root = tempfile::tempdir().unwrap();
    let studio = studio(root.path());
    for prefix in ["", "\u{feff}"] {
        let source = format!("{prefix}#!/usr/bin/env verum\nlet value: Int = 42;\nprint(value);\n");
        let valid = studio.call("language.request", json!({
            "path":"script.vr", "source":source,
        })).await.unwrap();
        assert_eq!(valid["result"]["parsed"], true, "{valid}");
        assert_eq!(valid["result"]["diagnostics"], json!([]), "{valid}");

        let malformed = format!("{prefix}#!/usr/bin/env verum\n// original line retained\nlet = ;\n");
        let invalid = studio.call("language.request", json!({
            "path":"script.vr", "source":malformed,
        })).await.unwrap();
        assert_eq!(invalid["result"]["parsed"], false, "{invalid}");
        assert_eq!(invalid["result"]["diagnostics"][0]["range"]["start"]["line"], 2, "{invalid}");
    }
}

#[tokio::test]
async fn closed_script_dependencies_use_the_same_source_parser() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("Verum.toml"), "[cog]\nname='script-project'\n").unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/dependency.vr"),
        "#!/usr/bin/env verum\npublic fn answer() -> Int { 42 }\nprint(answer());\n").unwrap();
    let studio = studio(root.path());
    let analysis = studio.call("language.request", json!({
        "path":"src/main.vr",
        "source":"mount script_project.dependency.{answer}; fn main() -> Int { answer() }",
    })).await.unwrap();
    assert_eq!(analysis["result"]["parsed"], true, "{analysis}");
    assert_eq!(analysis["result"]["diagnostics"], json!([]), "{analysis}");
    assert_eq!(analysis["result"]["bindings"][0]["name"], "answer");
}

#[tokio::test]
async fn actual_lsp_session_shares_revision_diagnostics_architecture_and_navigation() {
    let root = tempfile::tempdir().unwrap();
    let studio = studio(root.path());
    let source = "type Point is { x: Int, y: Int };\nfn identity(x: Int) -> Int { x }\nfn main() { let value = identity(42); }\n";
    let args = json!({"path":"main.vr", "source":source});
    let analysis = studio.call("language.request", args.clone()).await.unwrap();
    assert_eq!(analysis["result"]["parsed"], true);
    assert_eq!(analysis["result"]["source_hash"], analysis["source_hash"]);
    assert_eq!(
        analysis["result"]["declarations"].as_array().unwrap().len(),
        3
    );
    assert_eq!(analysis["result"]["diagnostics"], json!([]));
    let mut request = args.clone();
    request["method"] = json!("textDocument/definition");
    request["position"] = json!({"line":2,"character":28});
    let definition = studio.call("language.request", request).await.unwrap();
    assert_eq!(definition["source_hash"], analysis["source_hash"]);
    assert!(!definition["result"].is_null(), "{definition}");
    let error = studio
        .call(
            "language.request",
            json!({"path":"main.vr","source":"fn main() { let value: Int = true; }"}),
        )
        .await
        .unwrap();
    assert!(error["version"].as_i64().unwrap() > analysis["version"].as_i64().unwrap());
    assert!(
        !error["result"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let restored = studio.call("language.request", args).await.unwrap();
    assert_eq!(restored["source_hash"], analysis["source_hash"]);
    assert!(restored["version"].as_i64().unwrap() > error["version"].as_i64().unwrap());
    assert_eq!(restored["result"]["diagnostics"], json!([]));
    let project = studio.call("project.inspect", json!({})).await.unwrap();
    assert_eq!(
        project["analysis"]["coverage"],
        "open-documents-and-source-dependencies"
    );
    assert_eq!(
        project["analysis"]["documents"][0]["source_hash"],
        restored["source_hash"]
    );
    assert_eq!(
        project["analysis"]["documents"][0]["version"],
        restored["version"]
    );
}

#[tokio::test]
async fn context_requirements_are_projected_from_the_same_ast_and_exact_source_ranges() {
    let root = tempfile::tempdir().unwrap();
    let studio = studio(root.path());
    let source = "context Logger { fn log(message: Text); }\nfn report(value: Int) using [Logger] { print(value); }";
    let result = studio
        .call(
            "language.request",
            json!({"path":"contexts.vr","source":source}),
        )
        .await
        .unwrap();
    assert_eq!(result["result"]["parsed"], true, "{result}");
    let declarations = result["result"]["declarations"].as_array().unwrap();
    assert_eq!(declarations[0]["kind"], "context");
    assert_eq!(declarations[1]["requirements"][0]["source"], "Logger");
    assert_eq!(
        declarations[1]["requirements"][0]["range"]["start"]["line"],
        1
    );
}

#[tokio::test]
async fn proof_declarations_and_contracts_share_the_compiler_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let studio = studio(root.path());
    let source = concat!(
        "// Ω🙂 ranges must count UTF-16, not UTF-8 bytes\n",
        "theorem identity(A:Type,x:A):A proof=x;\n",
        "lemma implication(P:Type) requires P ensures P proof by assumption;\n",
        "corollary unchanged(A:Type,x:A):A proof=x;\n",
        "axiom assumed(A:Type) ensures A;\n",
        "fn guarded(x:Bool)->Bool requires x ensures result { x }\n",
        "fn plain(x:Bool)->Bool { x }\n",
    );
    let reply = studio.call("language.request", json!({"path":"proofs.vr","source":source}))
        .await.unwrap();
    let snapshot = &reply["result"];
    assert_eq!(snapshot["parsed"], true, "{reply}");
    assert_eq!(snapshot["source_hash"], reply["source_hash"]);
    let declarations = snapshot["declarations"].as_array().unwrap();
    let kinds: Vec<_> = declarations.iter().map(|item| item["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["theorem", "lemma", "corollary", "axiom", "function", "function"]);
    let source_at = |range: &Value| {
        let start = &range["start"];
        let end = &range["end"];
        assert_eq!(start["line"], end["line"]);
        let line = source.lines().nth(start["line"].as_u64().unwrap() as usize).unwrap();
        let utf16: Vec<_> = line.encode_utf16().collect();
        String::from_utf16(&utf16[start["character"].as_u64().unwrap() as usize..end["character"].as_u64().unwrap() as usize]).unwrap()
    };
    for item in declarations {
        assert_eq!(source_at(&item["selection_range"]), item["name"].as_str().unwrap());
        assert_eq!(item["requirements"], json!([]), "logical clauses are not context dependencies");
        for key in ["preconditions", "postconditions"] {
            for clause in item[key].as_array().unwrap() {
                assert_eq!(source_at(&clause["range"]), clause["source"].as_str().unwrap());
            }
        }
    }
    assert_eq!(declarations[1]["preconditions"][0]["source"], "P");
    assert_eq!(declarations[1]["postconditions"][0]["source"], "P");
    assert_eq!(declarations[4]["preconditions"][0]["source"], "x");
    assert_eq!(declarations[4]["postconditions"][0]["source"], "result");
    assert_eq!(declarations[5]["postconditions"], json!([]));
    let project = studio.call("project.inspect", json!({})).await.unwrap();
    assert_eq!(project["analysis"]["documents"][0]["declarations"], snapshot["declarations"],
        "Studio, agent tools and project architecture must use the same declarations");
}

#[tokio::test]
async fn saved_source_analysis_and_submitted_drafts_share_one_service_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let saved = "fn saved() -> Int { 7 }";
    std::fs::write(root.path().join("saved.vr"), saved).unwrap();
    let studio = studio(root.path());
    let from_disk = studio
        .call("language.request", json!({"path":"saved.vr"}))
        .await
        .unwrap();
    let draft = studio
        .call(
            "language.request",
            json!({"path":"saved.vr","source":"fn draft() -> Int { 9 }"}),
        )
        .await
        .unwrap();
    assert_ne!(from_disk["source_hash"], draft["source_hash"]);
    assert_eq!(
        std::fs::read_to_string(root.path().join("saved.vr")).unwrap(),
        saved
    );
    for path in ["../escape.vr", "/escape.vr", ".hidden.vr"] {
        assert!(
            studio
                .call("language.request", json!({"path":path,"source":saved}))
                .await
                .is_err()
        );
    }
    assert!(
        studio
            .call(
                "language.request",
                json!({"path":"saved.vr","method":"workspace/executeCommand","source":saved})
            )
            .await
            .is_err()
    );
    let tools = verum_workbench::codex::tools();
    assert!(
        tools
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "verum_analyze")
    );
    let _: Value = from_disk;
}

#[tokio::test]
async fn project_dependency_edits_reach_the_same_studio_and_agent_snapshot() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("Verum.toml"),
        "[cog]\nname='workspace-demo'\n",
    )
    .unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    let dependency = "public fn answer() -> Int { 42 }";
    std::fs::write(root.path().join("src/dependency.vr"), dependency).unwrap();
    let studio = studio(root.path());
    let args = json!({"path":"src/main.vr","source":"mount workspace_demo.dependency.{answer}; fn main() -> Int { answer() }"});
    let first = studio.call("language.request", args.clone()).await.unwrap();
    assert_eq!(first["result"]["diagnostics"], json!([]), "{first:#}");
    assert_eq!(first["result"]["bindings"][0]["name"], "answer");
    studio
        .call(
            "language.request",
            json!({"path":"src/dependency.vr","source":"public fn answer() -> Bool { true }"}),
        )
        .await
        .unwrap();
    let after = studio.call("language.request", args).await.unwrap();
    assert_eq!(after["version"], first["version"]);
    assert_ne!(
        after["result"]["project_revision"],
        first["result"]["project_revision"]
    );
    assert!(
        !after["result"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let project = studio.call("project.inspect", json!({})).await.unwrap();
    assert_eq!(
        project["analysis"]["project_revision"],
        after["result"]["project_revision"]
    );
    assert_eq!(project["analysis"]["sources"].as_array().unwrap().len(), 2);
    assert_eq!(
        std::fs::read_to_string(root.path().join("src/dependency.vr")).unwrap(),
        dependency
    );
}

#[tokio::test]
async fn architecture_uses_checked_record_dependencies_for_the_current_buffer() {
    let root = tempfile::tempdir().unwrap();
    let studio = studio(root.path());
    for (argument, dependencies) in [
        ("carrier", json!([0])),
        ("Int", json!([])),
        ("Missing", Value::Null),
    ] {
        let source = format!(
            "public type Witness<A> is {{ public value: A }};\n\
             public type Pack is {{ public carrier: Type, public witness: Witness<{argument}> }};"
        );
        let snapshot = studio.call("language.request", json!({
            "path": "records.vr", "source": source,
        })).await.unwrap();
        let record = snapshot["result"]["declarations"].as_array().unwrap().iter()
            .find(|item| item["name"] == "Pack").unwrap();
        let fields = record["record_fields"].as_array().unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0]["name"], "carrier");
        assert_eq!(fields[1]["annotation"]["source"], format!("Witness<{argument}>"));
        assert_eq!(fields[1]["depends_on"], dependencies, "{snapshot}");
        let start = &fields[0]["selection_range"]["start"];
        assert_eq!(start["line"], 1);
        assert_eq!(start["character"], source.lines().nth(1).unwrap().find("carrier").unwrap());
        let diagnostics = snapshot["result"]["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.iter().any(|d| d["severity"] == 1), dependencies.is_null());
    }
}
