//! T1651: real manifest parsing and the dispatched publication handler must
//! retain dependency intent. Loopback capture is not registry/consumer acceptance.

use super::{create_metadata, publish_with_signing_keys};
use crate::config::Manifest;
use crate::error::Result;
use crate::registry::CogMetadata;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use verum_common::{List, Map, Text};

const CHILD: &str = "cog::publication_dependencies::isolated_dependency_publication";
const CASE_ENV: &str = "VERUM_T1651_PUBLICATION_CASE";
const DETAILED: &str = r#"codec = { version = "^1.2", features = ["decode", "utf8"], optional = true, default-features = false }"#;

fn metadata(dependencies: &str) -> Result<CogMetadata> {
    let source = format!(
        "[cog]\nname = \"dependency-fixture\"\nversion = \"1.2.3\"\n[dependencies]\n{dependencies}\n"
    );
    let manifest: Manifest = toml::from_str(&source).unwrap();
    create_metadata(&manifest, "0".repeat(64), None)
}

#[test]
fn detailed_manifest_dependencies_reach_publication_metadata() {
    let published = metadata(DETAILED).unwrap();
    let json = serde_json::to_value(&published.dependencies).unwrap();
    assert_eq!(
        json["codec"],
        serde_json::json!({"version":"^1.2", "features":["decode", "utf8"],
                           "optional":true, "default_features":false}),
        "manifest options must survive both parsing and publication projection"
    );
}

#[test]
fn explicit_wildcards_and_unset_dependency_options_remain_distinct() {
    let published = metadata(
        r#"simple = "*"
table = { version = "*" }
empty = { version = "~2.1", features = [], optional = false, default-features = true }"#,
    )
    .unwrap();
    let json = serde_json::to_value(&published.dependencies).unwrap();
    assert_eq!(json["simple"], "*");
    assert_eq!(
        json["table"],
        serde_json::json!({"version":"*", "features":null,
                           "optional":null, "default_features":null})
    );
    assert_eq!(
        json["empty"],
        serde_json::json!({"version":"~2.1", "features":[],
                           "optional":false, "default_features":true})
    );
}

#[test]
fn unrepresentable_dependency_sources_are_not_registry_wildcards() {
    for fields in [
        r#"path = "../codec""#,
        r#"git = "https://invalid.example/codec""#,
        r#"git = "ipfs://unavailable-source""#,
        r#"version = "^1", path = "../codec""#,
        r#"version = "^1", git = "https://invalid.example/codec""#,
        r#"version = "^1", branch = "stable""#,
        r#"version = "^1", tag = "v1.2.0""#,
        r#"version = "^1", rev = "abc123""#,
    ] {
        let error = metadata(&format!("codec = {{ {fields} }}")).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("codec"), "{fields}: {message}");
        assert!(message.contains("source"), "{fields}: {message}");
    }
}

#[test]
fn omitted_or_invalid_dependency_versions_are_refused() {
    for declaration in [
        r#"codec = { features = ["decode"] }"#,
        "codec = {}",
        r#"codec = { version = "not-semver" }"#,
        r#"codec = "not-semver""#,
    ] {
        let error = metadata(declaration).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("codec"), "{declaration}: {message}");
        assert!(message.contains("version"), "{declaration}: {message}");
    }
}

#[test]
fn package_publish_transmits_original_manifest_dependency_options() {
    child_case("upload");
}

#[test]
fn unsupported_dependency_dry_run_refuses_before_archive_creation() {
    child_case("refuse");
}

fn child_case(case: &str) {
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--nocapture"])
        .env(CASE_ENV, case)
        .env("VERUM_REGISTRY_TOKEN", "fixture-dependency-token")
        .env("NO_PROXY", "127.0.0.1")
        .env("no_proxy", "127.0.0.1")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .output()
        .unwrap();
    let output = format!(
        "{}\n{}",
        std::str::from_utf8(&result.stdout).unwrap(),
        std::str::from_utf8(&result.stderr).unwrap()
    );
    assert!(result.status.success(), "{case}: {output}");
    if case == "refuse" {
        assert!(!output.contains("valid for publishing"), "{output}");
        assert!(!output.contains("Created cog archive"), "{output}");
    }
}

#[test]
fn isolated_dependency_publication() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let project = TempDir::new().unwrap();
    std::env::set_current_dir(project.path()).unwrap();
    let name = format!("dependency-publication-{}", std::process::id());
    let archive_path = std::env::temp_dir().join(format!("{name}-1.2.3.vr"));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let registry = format!("http://{}/private", listener.local_addr().unwrap());
    let dependencies = if case == "refuse" {
        r#"codec = { path = "../codec" }"#
    } else {
        DETAILED
    };
    let manifest_bytes = format!(
        "[cog]\nname = \"{name}\"\nversion = \"1.2.3\"\n\
         [registry]\nindex = \"{registry}\"\n\
         [dependencies]\n{dependencies}\n\
         [features]\nexpanded = [\"codec\"]\n"
    );
    std::fs::write(project.path().join("verum.toml"), &manifest_bytes).unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    let source_bytes = b"module dependency_publication.lib;\npublic fn answer() -> Int { 42 }\n";
    std::fs::write(project.path().join("src/lib.vr"), source_bytes).unwrap();

    if case == "refuse" {
        let result = publish_with_signing_keys(true, true, &[]);
        let archive_created = archive_path.exists();
        let _ = std::fs::remove_file(&archive_path);
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("codec") && message.contains("source"),
            "{message}"
        );
        assert!(
            !archive_created,
            "invalid dependencies must fail before creating an archive"
        );
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        return;
    }

    let worker = thread::spawn(move || capture_publication(listener));
    let result = publish_with_signing_keys(false, true, &[]);
    let capture = worker.join().expect("publication capture");
    let _ = std::fs::remove_file(&archive_path);
    result.expect("loopback publication must complete through receipt validation");
    assert_eq!(capture.metadata["name"], name);
    assert_eq!(
        capture.metadata["dependencies"]["codec"],
        serde_json::json!({"version":"^1.2", "features":["decode", "utf8"],
                           "optional":true, "default_features":false})
    );
    assert_eq!(
        capture.metadata["features"]["expanded"],
        serde_json::json!(["codec"])
    );

    // This path uses the real archive builder. Retaining its original manifest
    // distinguishes faithful metadata from rewriting the declaration to match it.
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(capture.archive.as_slice()));
    let mut entries = Map::<Text, List<u8>>::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        let mut contents = List::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let count = entry.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            contents.extend_from_slice(&buffer[..count]);
        }
        entries.insert(path.into(), contents);
    }
    assert_eq!(
        entries[&Text::from("verum.toml")].as_slice(),
        manifest_bytes.as_bytes()
    );
    assert_eq!(entries[&Text::from("src/lib.vr")].as_slice(), source_bytes);
}

struct PublicationCapture {
    metadata: serde_json::Value,
    archive: List<u8>,
}

fn capture_publication(listener: TcpListener) -> PublicationCapture {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "publication client did not connect"
                );
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("fixture accept: {error}"),
        }
    };
    // Accepted sockets inherit O_NONBLOCK on Darwin; timeouts do not clear it.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let body = request_body(&mut stream);
    let body = body.as_slice();
    let metadata_len = u32::from_le_bytes(body[..4].try_into().unwrap()) as usize;
    let metadata: serde_json::Value = serde_json::from_slice(&body[4..4 + metadata_len]).unwrap();
    let archive_len = u64::from_le_bytes(
        body[4 + metadata_len..12 + metadata_len]
            .try_into()
            .unwrap(),
    );
    let archive = &body[12 + metadata_len..];
    assert_eq!(archive.len() as u64, archive_len);
    assert_eq!(metadata["checksum"], hex::encode(Sha256::digest(archive)));
    let receipt = serde_json::to_vec(&serde_json::json!({
        "name": metadata["name"], "version": metadata["version"], "checksum": metadata["checksum"],
    }))
    .unwrap();
    write!(stream, "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", receipt.len()).unwrap();
    stream.write_all(&receipt).unwrap();
    PublicationCapture {
        metadata,
        archive: archive.iter().copied().collect(),
    }
}

fn request_body(stream: &mut TcpStream) -> List<u8> {
    let mut bytes = List::new();
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
        let count = stream.read(&mut buffer).expect("publication header read");
        assert!(count > 0, "publication ended before headers");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 16 * 1024, "fixture header bound");
    };
    let header = std::str::from_utf8(&bytes.as_slice()[..header_end]).unwrap();
    let mut lines = header.lines();
    assert_eq!(
        lines.next(),
        Some("POST /private/api/v1/cogs/publish HTTP/1.1")
    );
    let headers: Map<Text, Text> = lines
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (name, value) = line.split_once(':').unwrap();
            (name.to_ascii_lowercase().into(), value.trim().into())
        })
        .collect();
    assert_eq!(
        headers[&Text::from("authorization")],
        "Bearer fixture-dependency-token"
    );
    assert_eq!(
        headers[&Text::from("content-type")],
        "application/vnd.verum.cog-publish.v1"
    );
    let length: usize = headers[&Text::from("content-length")].parse().unwrap();
    assert!(length <= 64 * 1024, "fixture body bound");
    while bytes.len() - header_end < length {
        let count = stream.read(&mut buffer).expect("publication body read");
        assert!(count > 0, "truncated publication body");
        bytes.extend_from_slice(&buffer[..count]);
    }
    assert_eq!(bytes.len() - header_end, length);
    bytes.as_slice()[header_end..].iter().copied().collect()
}
