//! T1642: exercise the actual RegistryClient over controlled loopback HTTP.
//! These fixtures validate the client contract, not Registry deployment readiness.

use crate::config::Manifest;
use crate::registry::RegistryClient;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use verum_common::Text;

const CHILD: &str = "registry::client::index_contract::isolated_index_contract";
const CASE_ENV: &str = "VERUM_INDEX_CONTRACT_CASE";
const LIMIT: usize = 1024 * 1024;

#[test]
fn validates_the_whole_index_before_resolution() {
    for case in [
        "valid",
        "wrong-name",
        "array-index",
        "array-version",
        "bad-version",
        "duplicate-version",
        "conflicting-version",
        "empty-checksum",
        "bad-checksum",
        "uppercase-checksum",
        "features-array",
        "feature-nonstring",
        "duplicate-feature-key",
        "missing-features",
        "missing-yanked",
        "nonbool-yanked",
        "top-level-duplicate-key",
        "duplicate-version-field",
        "yanked-preserved",
        "build-metadata-identities",
        "bad-yanked-version",
        "unknown-index-field",
        "unknown-version-field",
    ] {
        child(case);
    }
}

#[test]
fn enforces_body_and_version_budgets_before_accepting_an_index() {
    for case in [
        "exact-byte-limit",
        "over-byte-limit",
        "over-byte-limit-no-length",
        "versions-1024",
        "versions-1025",
        "503",
        "partial-206",
        "relative-redirect",
        "origin-redirect",
        "empty-200",
        "empty-versions",
        "truncated-json",
    ] {
        child(case);
    }
}

#[test]
fn preserves_manifest_prefix_and_encodes_one_valid_name_segment() {
    for case in [
        "base-trailing-slash",
        "base-query",
        "base-fragment",
        "unicode-name",
        "max-name",
        "slash-name",
        "query-name",
        "fragment-name",
        "percent-name",
        "traversal-name",
        "control-name",
        "empty-name",
        "overlong-name",
    ] {
        child(case);
    }
}

fn child(case: &str) {
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--nocapture"])
        .env(CASE_ENV, case)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{case}: {}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

fn entry(version: &str) -> Value {
    json!({"version": version, "checksum": "a".repeat(64), "yanked": false,
           "features": {"default": ["net", "net"], "net": []}})
}

fn response_body(case: &str, name: &str) -> String {
    let mut body = json!({"name": name, "versions": [entry("1.0.0"), entry("1.1.0")]});
    match case {
        "wrong-name" => body["name"] = json!("another"),
        "array-index" => body = json!([name, [entry("1.0.0")]]),
        "array-version" => {
            body["versions"][1] = json!(["1.1.0", "a".repeat(64), false, {}]);
        }
        "empty-versions" => body["versions"] = json!([]),
        "bad-version" => body["versions"][1]["version"] = json!("invalid"),
        "bad-yanked-version" => {
            body["versions"][1]["version"] = json!("invalid");
            body["versions"][1]["yanked"] = json!(true);
        }
        "build-metadata-identities" => {
            body["versions"] = json!([entry("1.0.0+one"), entry("1.0.0+two")]);
        }
        "unknown-index-field" => body["partial"] = json!(true),
        "unknown-version-field" => body["versions"][1]["partial"] = json!(true),
        "duplicate-version" => body["versions"][1] = body["versions"][0].clone(),
        "conflicting-version" => {
            body["versions"][1] = body["versions"][0].clone();
            body["versions"][1]["checksum"] = json!("b".repeat(64));
        }
        "empty-checksum" => body["versions"][1]["checksum"] = json!(""),
        "bad-checksum" => body["versions"][1]["checksum"] = json!("g".repeat(64)),
        "uppercase-checksum" => body["versions"][1]["checksum"] = json!("A".repeat(64)),
        "features-array" => body["versions"][1]["features"] = json!([]),
        "missing-yanked" => {
            body["versions"][1]
                .as_object_mut()
                .unwrap()
                .remove("yanked");
        }
        "nonbool-yanked" => body["versions"][1]["yanked"] = json!("false"),
        "yanked-preserved" => body["versions"][1]["yanked"] = json!(true),
        "feature-nonstring" => body["versions"][1]["features"] = json!({"net": [7]}),
        "missing-features" => {
            body["versions"][1]
                .as_object_mut()
                .unwrap()
                .remove("features");
        }
        "versions-1024" | "versions-1025" => {
            let count = if case == "versions-1024" { 1024 } else { 1025 };
            body["versions"] =
                Value::Array((0..count).map(|i| entry(&format!("1.0.{i}"))).collect());
        }
        _ => {}
    }
    let mut text = body.to_string();
    match case {
        "top-level-duplicate-key" => {
            return format!(
                "{{\"name\":\"other\",\"name\":\"widget\",\"versions\":[{}]}}",
                entry("1.0.0")
            );
        }
        "duplicate-version-field" => {
            let row = entry("1.0.0").to_string().replacen(
                "\"version\":\"1.0.0\"",
                "\"version\":\"9.0.0\",\"version\":\"1.0.0\"",
                1,
            );
            return format!("{{\"name\":\"widget\",\"versions\":[{row}]}}");
        }
        "duplicate-feature-key" => {
            return format!(
                "{{\"name\":\"widget\",\"versions\":[{{\"version\":\"1.0.0\",\"checksum\":\"{}\",\"yanked\":false,\"features\":{{\"net\":[\"a\"],\"net\":[\"b\"]}}}}]}}",
                "a".repeat(64)
            );
        }
        "exact-byte-limit" => text.extend(std::iter::repeat_n(' ', LIMIT - text.len())),
        "over-byte-limit" | "over-byte-limit-no-length" => {
            text.extend(std::iter::repeat_n(' ', LIMIT + 1 - text.len()))
        }
        "empty-200" => return "".into(),
        "truncated-json" => {
            text.pop();
        }
        _ => {}
    }
    text
}

fn request_line(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        let mut chunk = [0; 1024];
        let n = stream.read(&mut chunk).unwrap();
        assert!(n > 0 && bytes.len() + n <= 16384);
        bytes.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string()
}

#[test]
fn isolated_index_contract() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let project = TempDir::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    // One subprocess owns all environment/current-directory changes. An
    // accidental request outside the configured origin reaches this guard.
    unsafe {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            std::env::set_var(key, &origin);
        }
        for key in ["NO_PROXY", "no_proxy"] {
            std::env::set_var(key, "127.0.0.1,localhost");
        }
    }
    std::env::set_current_dir(project.path()).unwrap();
    let configured = match case.as_str() {
        "base-trailing-slash" => format!("{origin}/private/"),
        "base-query" => format!("{origin}/private?query=1"),
        "base-fragment" => format!("{origin}/private#fragment"),
        _ => format!("{origin}/private"),
    };
    std::fs::write(
        Manifest::manifest_path(project.path()),
        format!(
            "[cog]\nname=\"consumer\"\nversion=\"1.0.0\"\n[registry]\nindex=\"{configured}\"\n"
        ),
    )
    .unwrap();
    let name = match case.as_str() {
        "unicode-name" => "café".to_owned(),
        "max-name" => "w".repeat(4096),
        "slash-name" => "widget/1.0.0/download".to_owned(),
        "query-name" => "widget?force=1".to_owned(),
        "fragment-name" => "widget#ignored".to_owned(),
        "percent-name" => "%2F".to_owned(),
        "traversal-name" => "..".to_owned(),
        "control-name" => "widget\nheader".to_owned(),
        "empty-name" => String::new(),
        "overlong-name" => "w".repeat(4097),
        _ => "widget".to_owned(),
    };
    let invalid_name = matches!(
        case.as_str(),
        "slash-name"
            | "query-name"
            | "fragment-name"
            | "percent-name"
            | "traversal-name"
            | "control-name"
            | "empty-name"
            | "overlong-name"
    );
    let body = response_body(&case, &name);
    let no_length = case == "over-byte-limit-no-length";
    let status = match case.as_str() {
        "503" => "503 Service Unavailable",
        "partial-206" => "206 Partial Content",
        "relative-redirect" | "origin-redirect" => "302 Found",
        _ => "200 OK",
    };
    let location = match case.as_str() {
        "relative-redirect" => "Location: /replacement-index\r\n".to_owned(),
        "origin-redirect" => format!("Location: {origin}/replacement-index\r\n"),
        _ => String::new(),
    };
    let (stop, stopped) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut requests = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(35);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    requests.push(request_line(&mut stream));
                    let length = if no_length {
                        String::new()
                    } else {
                        format!("Content-Length: {}\r\n", body.len())
                    };
                    let header = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{length}{location}Connection: close\r\n\r\n"
                    );
                    stream.write_all(header.as_bytes()).unwrap();
                    // Correct bounded readers may close before consuming an
                    // oversized body. That transport result is expected here.
                    let _ = stream.write_all(body.as_bytes());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if stopped.try_recv().is_ok() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        }
        requests
    });
    let client = RegistryClient::from_manifest().unwrap();
    assert_eq!(client.base_url(), configured);
    let result = client.get_index(&name);
    let _ = stop.send(());
    let requests = worker.join().unwrap();
    if invalid_name || matches!(case.as_str(), "base-query" | "base-fragment") {
        assert!(
            requests.is_empty(),
            "invalid registry coordinate reached HTTP: {case}: {requests:?}"
        );
    } else {
        let segment = if case == "unicode-name" {
            "caf%C3%A9"
        } else {
            &name
        };
        assert_eq!(
            requests,
            [format!("GET /private/api/v1/cogs/{segment}/index HTTP/1.1")]
        );
    }
    let valid = matches!(
        case.as_str(),
        "valid"
            | "exact-byte-limit"
            | "versions-1024"
            | "base-trailing-slash"
            | "unicode-name"
            | "max-name"
            | "yanked-preserved"
            | "build-metadata-identities"
    );
    assert_eq!(result.is_ok(), valid, "{case}: {result:?}");
    if case == "build-metadata-identities" {
        let index = result.unwrap();
        assert_eq!(index.versions.len(), 2);
        assert_eq!(index.versions[0].version.as_str(), "1.0.0+one");
        assert_eq!(index.versions[1].version.as_str(), "1.0.0+two");
    } else if case == "valid" || case == "yanked-preserved" {
        let index = result.unwrap();
        assert_eq!(index.name.as_str(), "widget");
        assert_eq!(index.versions.len(), 2);
        assert!(!index.versions[0].yanked);
        assert_eq!(index.versions[1].yanked, case == "yanked-preserved");
        let values = index.versions[0]
            .features
            .get(&Text::from("default"))
            .unwrap();
        assert_eq!(values.len(), 2); // duplicate array values must be retained
        assert_eq!(values[0].as_str(), "net");
        assert_eq!(values[1].as_str(), "net");
    }
}
