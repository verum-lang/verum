//! Exercise the shipped metadata through Studio's actual embedded LSP session.
use serde_json::json;
use verum_common::List;
use verum_lsp::library_metadata::LibraryMetadata;
use verum_workbench::{Config, Studio};

fn release_studio() -> (tempfile::TempDir, Studio) {
    let artifact = std::env::var_os("VERUM_TEST_LIBRARY_METADATA").expect("release metadata path");
    let library = LibraryMetadata::load(std::path::Path::new(&artifact)).unwrap();
    eprintln!("library {}", library.identity.sha256);
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::open_with_library(
        Config {
            workspace: root.path().to_path_buf(),
            verum: "verum".into(),
            proof: "verum-source-replay".into(),
            codex: "codex".into(),
            port: 0,
        },
        Some(library),
    )
    .unwrap();
    (root, studio)
}

#[tokio::test]
#[ignore = "requires VERUM_TEST_LIBRARY_METADATA from the matching release build"]
async fn release_library_imports_keep_their_declared_owners() {
    let (_workspace, studio) = release_studio();
    for mount in [
        "core.base.protocols.{Display}",
        "core.base.{Display}",
        "core.base.{Display as Printable}",
        "core.{Display}",
        "core.*",
    ] {
        let source = format!("mount {mount}; fn main() -> Int {{ 42 }}");
        let analysis = studio
            .call(
                "language.request",
                json!({"path":"main.vr", "source":source}),
            )
            .await
            .unwrap();
        assert_eq!(analysis["result"]["parsed"], true, "{mount}: {analysis}");
        let errors: List<_> = analysis["result"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["severity"] == 1)
            .collect();
        assert!(errors.is_empty(), "{mount}: {errors:?}");
    }
    for mount in [
        "core.hash.crypto.sha512.{K}",
        "core.base.{__MissingPublicExport__}",
    ] {
        let analysis = studio
            .call(
                "language.request",
                json!({
                    "path":"main.vr", "source":format!("mount {mount}; fn main() -> Int {{ 42 }}"),
                }),
            )
            .await
            .unwrap();
        assert!(
            analysis["result"]["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["severity"] == 1),
            "private or absent declaration was accepted: {mount}: {analysis}"
        );
    }
    for (source, valid) in [
        ("fn bytes(t: &Text) -> &[Byte] { t.as_bytes() }", true),
        ("fn owned(t: Text) -> List<Byte> { t.into_bytes() }", true),
        ("fn wrong(t: &Text) -> Bool { t.as_bytes() }", false),
        ("type Packet is { n: Int }; fn wrong(p: Packet) -> Int { p.as_bytes().len() }", false),
    ] {
        let analysis = studio.call("language.request", json!({"path":"main.vr", "source":source}))
            .await.unwrap();
        assert_eq!(analysis["result"]["parsed"], true, "{source}");
        let diagnostics = analysis["result"]["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.iter().all(|d| d["severity"] != 1), valid,
            "{source}: {diagnostics:?}");
    }
}

#[tokio::test]
#[ignore = "requires VERUM_TEST_LIBRARY_METADATA with compiled scalar declarations"]
async fn compiled_scalar_methods_keep_their_array_extents() {
    let (_workspace, studio) = release_studio();
    for (source, expected_error) in [
        ("fn bytes(n: UInt64) -> [Byte; 8] { let bits = n * 8_u64; bits.to_be_bytes() }", None),
        ("fn bytes(n: UInt16) -> [Byte; 2] { n.to_le_bytes() }", None),
        ("fn bytes(n: UInt16) -> [UInt8; 2] { n.to_le_bytes() }", None),
        ("fn wrong(n: UInt16) -> Bool { n.to_le_bytes() }", Some(("E400", "found '[Byte; 2]'"))),
        ("fn wrong(n: UInt64) -> [Byte; 7] { n.to_be_bytes() }", Some(("E413", "expected 7, found 8"))),
    ] {
        let analysis = studio
            .call("language.request", json!({"path":"main.vr", "source":source}))
            .await
            .unwrap();
        assert_eq!(analysis["result"]["parsed"], true, "{source}");
        let errors: Vec<_> = analysis["result"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["severity"] == 1)
            .collect();
        match expected_error {
            None => assert!(errors.is_empty(), "{source}: {errors:?}"),
            Some((code, message)) => {
                assert_eq!(errors.len(), 1, "{source}: {errors:?}");
                assert_eq!(errors[0]["code"], code, "{source}: {errors:?}");
                assert!(errors[0]["message"].as_str().unwrap().contains(message), "{source}: {errors:?}");
            }
        }
    }
}
