//! T1634: the install flow must keep metadata, archive and lockfile on one registry.
//!
//! Run each cwd/proxy-sensitive case in a fresh test process. The proxy is a
//! loopback refusal fixture, so an accidental public-registry URL is observable
//! without allowing the regression to contact the public service.

use super::install_from_registry;
use crate::config::Manifest;
use crate::registry::{CacheManager, CogSource, Lockfile, RegistryClient};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use verum_common::{List, Text};

const CHILD_TEST: &str = "cog::configured_registry_downloads::isolated_install_case";
const CASE_ENV: &str = "VERUM_T1634_INSTALL_CASE";
const ARCHIVE: &[u8] = b"configured registry artifact\0\xff";

#[test]
fn install_uses_configured_registry_for_metadata_archive_and_lockfile() {
    child_case("success");
}

#[test]
fn archive_failure_stays_on_configured_registry() {
    child_case("archive-failure");
}

fn child_case(case: &str) {
    let result = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .env(CASE_ENV, case)
        .output()
        .expect("spawn isolated install test");
    assert!(
        result.status.success(),
        "case {case} failed:\nstdout:\n{}\nstderr:\n{}",
        std::str::from_utf8(&result.stdout).unwrap_or("non-UTF-8 stdout"),
        std::str::from_utf8(&result.stderr).unwrap_or("non-UTF-8 stderr"),
    );
}

#[test]
fn isolated_install_case() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let project = TempDir::new().expect("temporary project");
    let cache_dir = TempDir::new().expect("temporary package cache");
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback fixture");
    listener.set_nonblocking(true).unwrap();
    let origin: Text = format!("http://{}", listener.local_addr().unwrap()).into();
    let configured: Text = format!("{origin}/private").into();

    // SAFETY: this fresh subprocess runs only this test. Set its transport
    // configuration before constructing clients or starting the HTTP worker.
    unsafe {
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"] {
            std::env::set_var(key, origin.as_str());
        }
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
        std::env::set_var("no_proxy", "127.0.0.1,localhost");
    }
    std::env::set_current_dir(project.path()).unwrap();
    let manifest_path = project.path().join(Manifest::MANIFEST_FILENAME);
    std::fs::write(
        &manifest_path,
        format!(
            "[cog]\nname = \"consumer\"\nversion = \"0.1.0\"\n\n[registry]\nindex = \"{configured}\"\n"
        ),
    )
    .unwrap();
    let before = std::fs::read(&manifest_path).unwrap();
    let checksum: Text = format!("{:x}", Sha256::digest(ARCHIVE)).into();
    let metadata = serde_json::json!({
        "name": "fixture", "version": "1.2.3", "description": null,
        "authors": [], "license": null, "repository": null, "homepage": null,
        "keywords": [], "categories": [], "readme": null,
        "dependencies": {}, "features": {}, "artifacts": {},
        "proofs": null, "cbgr_profiles": null, "signature": null,
        "ipfs_hash": null, "checksum": checksum.as_str(), "published_at": 1
    });
    let metadata: Text = serde_json::to_string(&metadata).unwrap().into();
    let archive_failure = case == "archive-failure";
    let (stop, stopping) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut requests: List<Text> = List::new();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let request = read_request(&mut stream);
                    let first = request.lines().next().unwrap_or("");
                    requests.push(first.into());
                    assert!(
                        !request.lines().any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
                        "install must not send publish credentials: {first}",
                    );
                    let (status, content_type, body) = match first {
                        "GET /private/api/v1/cogs/fixture/latest HTTP/1.1" =>
                            ("200 OK", "application/json", &b"{\"version\":\"1.2.3\"}"[..]),
                        "GET /private/api/v1/cogs/fixture/1.2.3 HTTP/1.1" =>
                            ("200 OK", "application/json", metadata.as_bytes()),
                        "GET /private/api/v1/security/vulnerabilities/fixture/1.2.3 HTTP/1.1" =>
                            ("200 OK", "application/json", &b"{\"package\":\"fixture\",\"version\":\"1.2.3\",\"vulnerabilities\":[]}"[..]),
                        "GET /private/api/v1/cogs/fixture/1.2.3/download HTTP/1.1" if archive_failure =>
                            ("503 Service Unavailable", "text/plain", &b"archive unavailable"[..]),
                        "GET /private/api/v1/cogs/fixture/1.2.3/download HTTP/1.1" =>
                            ("200 OK", "application/octet-stream", ARCHIVE),
                        _ => ("502 Bad Gateway", "text/plain", &b"unexpected registry destination"[..]),
                    };
                    let header = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len(),
                    );
                    stream.write_all(header.as_bytes()).unwrap();
                    stream.write_all(body).unwrap();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if stopping.try_recv().is_ok() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture accept failed: {error}"),
            }
        }
        requests
    });
    let client = RegistryClient::from_manifest().expect("configured client");
    let cache = CacheManager::new(cache_dir.path().to_path_buf()).unwrap();
    let result = install_from_registry("fixture", None, &client, &cache);
    stop.send(()).unwrap();
    let requests = worker.join().expect("fixture worker");
    assert_eq!(
        requests.iter().map(Text::as_str).collect::<List<_>>(),
        List::from([
            "GET /private/api/v1/cogs/fixture/latest HTTP/1.1",
            "GET /private/api/v1/cogs/fixture/1.2.3 HTTP/1.1",
            "GET /private/api/v1/security/vulnerabilities/fixture/1.2.3 HTTP/1.1",
            "GET /private/api/v1/cogs/fixture/1.2.3/download HTTP/1.1",
        ]),
        "metadata and artifact must stay on the selected registry; result: {result:?}",
    );
    if archive_failure {
        assert!(result.is_err(), "a failed archive transfer must fail installation");
        assert_eq!(std::fs::read(&manifest_path).unwrap(), before);
        assert!(!Manifest::lockfile_path(project.path()).exists());
    } else {
        result.expect("install from selected registry");
        assert_eq!(std::fs::read(cache.get_cog_path("fixture", "1.2.3")).unwrap(), ARCHIVE);
        let lock = Lockfile::from_file(&Manifest::lockfile_path(project.path())).unwrap();
        let entry = lock.packages.iter().find(|entry| entry.name.as_str() == "fixture").unwrap();
        assert!(
            matches!(&entry.source, CogSource::Registry { registry, version }
                if registry == &configured && version.as_str() == "1.2.3"),
            "lockfile must retain the selected registry: {:?}",
            entry.source,
        );
        let manifest = Manifest::from_file(&manifest_path).unwrap();
        assert!(manifest.dependencies.contains_key(&Text::from("fixture")));
    }
}

fn read_request(stream: &mut TcpStream) -> Text {
    let mut bytes: List<u8> = List::new();
    let mut buf = [0_u8; 1024];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buf).expect("read fixture request");
        assert!(read > 0, "request ended before headers");
        bytes.extend_from_slice(&buf[..read]);
        assert!(bytes.len() < 16384, "fixture request headers too large");
    }
    std::str::from_utf8(&bytes).expect("HTTP headers").into()
}
