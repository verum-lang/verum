//! T1634/T1673: keep install requests on one registry and require an actual advisory report.
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

#[test]
fn malformed_manifest_does_not_select_public_registry() {
    child_case("invalid-manifest");
}

#[test]
fn existing_implicit_registry_defaults_are_preserved() {
    child_case("defaults");
}

const ADVISORY_CASES: &[&str] = &[
    "empty",
    "nonempty",
    "401",
    "403",
    "404",
    "429",
    "500",
    "503",
    "malformed",
    "wrong-package",
    "wrong-version",
    "transport",
];

#[test]
fn advisory_client_distinguishes_reports_from_unavailable_data() {
    for case in ADVISORY_CASES {
        child_case(&format!("direct-advisory-{case}"));
    }
}

#[test]
fn unavailable_advisories_stop_install_before_download_or_project_mutation() {
    for case in ADVISORY_CASES {
        child_case(&format!("install-advisory-{case}"));
    }
}

fn advisory_response(case: &str) -> (&'static str, &'static [u8], bool) {
    let empty = &b"{\"package\":\"fixture\",\"version\":\"1.2.3\",\"vulnerabilities\":[]}"[..];
    match case {
        "empty" => ("200 OK", empty, false),
        "nonempty" => ("200 OK", br#"{"package":"fixture","version":"1.2.3","vulnerabilities":[{"id":"FIXTURE-1","severity":"high","title":"Fixture advisory","description":"fixture vulnerability detail","patched_versions":["1.2.4"]}]}"#, false),
        "401" => ("401 Unauthorized", empty, false),
        "403" => ("403 Forbidden", empty, false),
        "404" => ("404 Not Found", empty, false),
        "429" => ("429 Too Many Requests", empty, false),
        "500" => ("500 Internal Server Error", empty, false),
        "503" => ("503 Service Unavailable", empty, false),
        "malformed" => ("200 OK", b"{not a report", false),
        "wrong-package" => ("200 OK", br#"{"package":"other","version":"1.2.3","vulnerabilities":[]}"#, false),
        "wrong-version" => ("200 OK", br#"{"package":"fixture","version":"9.9.9","vulnerabilities":[]}"#, false),
        "transport" => ("200 OK", empty, true),
        _ => panic!("unknown advisory fixture {case}"),
    }
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
    if let Some(advisory) = case.strip_prefix("install-advisory-") {
        let stdout = std::str::from_utf8(&result.stdout).unwrap();
        let stderr = std::str::from_utf8(&result.stderr).unwrap();
        let output = format!("{stdout}\n{stderr}");
        assert_eq!(
            output.contains("No known vulnerabilities"),
            advisory == "empty",
            "only a valid empty report supports the clean message: {case}: {output}"
        );
        if advisory == "nonempty" {
            assert!(output.contains("fixture vulnerability detail"), "{output}");
            assert!(output.contains("Installed"), "{output}");
        } else if advisory != "empty" {
            assert!(!output.contains("Downloading cog"), "{output}");
            assert!(!output.contains("Installed"), "{output}");
        }
    }
}

#[test]
fn isolated_install_case() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let project = TempDir::new().expect("temporary project");
    if case == "invalid-manifest" || case == "defaults" {
        std::env::set_current_dir(project.path()).unwrap();
        let path = project.path().join(Manifest::MANIFEST_FILENAME);
        if case == "invalid-manifest" {
            std::fs::write(&path, "[cog\nname = broken").unwrap();
            assert!(matches!(
                RegistryClient::from_manifest(),
                Err(crate::error::CliError::ConfigParse(_))
            ));
        } else {
            assert_eq!(
                RegistryClient::from_manifest().unwrap().base_url(),
                crate::registry::DEFAULT_REGISTRY,
            );
            std::fs::write(&path, "[cog]\nname = \"consumer\"\nversion = \"0.1.0\"\n").unwrap();
            assert_eq!(
                RegistryClient::from_manifest().unwrap().base_url(),
                crate::config::RegistryConfig::default().index.as_str(),
            );
        }
        return;
    }
    let cache_dir = TempDir::new().expect("temporary package cache");
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback fixture");
    listener.set_nonblocking(true).unwrap();
    let origin: Text = format!("http://{}", listener.local_addr().unwrap()).into();
    let configured: Text = format!("{origin}/private").into();

    // SAFETY: this fresh subprocess runs only this test. Set its transport
    // configuration before constructing clients or starting the HTTP worker.
    unsafe {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
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
    let checksum: Text = hex::encode(Sha256::digest(ARCHIVE)).into();
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
    let direct_advisory = case.starts_with("direct-advisory-");
    let advisory_case = case
        .strip_prefix("direct-advisory-")
        .or_else(|| case.strip_prefix("install-advisory-"));
    let advisory_unavailable =
        advisory_case.is_some_and(|kind| kind != "empty" && kind != "nonempty");
    let (advisory_status, advisory_body, disconnect) =
        advisory_response(advisory_case.unwrap_or("empty"));
    let (stop, stopping) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut requests: List<Text> = List::new();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    // Darwin can inherit the listener's nonblocking mode.
                    // These bounded request reads require blocking streams.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let request = read_request(&mut stream);
                    let first = request.as_str().lines().next().unwrap_or("");
                    requests.push(first.into());
                    assert!(
                        !request
                            .as_str()
                            .lines()
                            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
                        "install must not send publish credentials: {first}",
                    );
                    if disconnect
                        && first
                            == "GET /private/api/v1/security/vulnerabilities/fixture/1.2.3 HTTP/1.1"
                    {
                        // A real peer closes without a response; the client must
                        // propagate transport failure, never synthesize a report.
                        continue;
                    }
                    let (status, content_type, body) = match first {
                        "GET /private/api/v1/cogs/fixture/latest HTTP/1.1" => (
                            "200 OK",
                            "application/json",
                            &b"{\"version\":\"1.2.3\"}"[..],
                        ),
                        "GET /private/api/v1/cogs/fixture/1.2.3 HTTP/1.1" => {
                            ("200 OK", "application/json", metadata.as_bytes())
                        }
                        "GET /private/api/v1/security/vulnerabilities/fixture/1.2.3 HTTP/1.1" => {
                            (advisory_status, "application/json", advisory_body)
                        }
                        "GET /private/api/v1/cogs/fixture/1.2.3/download HTTP/1.1"
                            if archive_failure =>
                        {
                            (
                                "503 Service Unavailable",
                                "text/plain",
                                &b"archive unavailable"[..],
                            )
                        }
                        "GET /private/api/v1/cogs/fixture/1.2.3/download HTTP/1.1" => {
                            ("200 OK", "application/octet-stream", ARCHIVE)
                        }
                        _ => (
                            "502 Bad Gateway",
                            "text/plain",
                            &b"unexpected registry destination"[..],
                        ),
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
    if direct_advisory {
        let report = client.check_vulnerabilities("fixture", "1.2.3");
        stop.send(()).unwrap();
        let requests = worker.join().expect("fixture worker");
        assert!(!requests.is_empty());
        assert!(requests.iter().all(|request| request.as_str()
            == "GET /private/api/v1/security/vulnerabilities/fixture/1.2.3 HTTP/1.1"));
        if advisory_unavailable {
            let error = report.expect_err("unavailable or unrelated data is not a report");
            match advisory_case.unwrap() {
                "transport" => assert!(matches!(error, crate::error::CliError::Network(_))),
                "malformed" | "wrong-package" | "wrong-version" => {
                    assert!(matches!(error, crate::error::CliError::Registry(_)))
                }
                status => assert!(error.to_string().contains(status), "{error}"),
            }
        } else {
            let report = report.expect("valid advisory report");
            assert_eq!(report.package.as_str(), "fixture");
            assert_eq!(report.version.as_str(), "1.2.3");
            assert_eq!(
                report.vulnerabilities.len(),
                usize::from(advisory_case == Some("nonempty"))
            );
            if advisory_case == Some("nonempty") {
                assert_eq!(report.vulnerabilities[0].id.as_str(), "FIXTURE-1");
            }
        }
        return;
    }
    let result = install_from_registry("fixture", None, &client, &cache);
    stop.send(()).unwrap();
    let requests = worker.join().expect("fixture worker");
    if advisory_unavailable {
        assert!(
            result.is_err(),
            "advisory failure must stop installation: {advisory_case:?}"
        );
        assert!(requests.len() >= 3);
        assert_eq!(
            requests[0].as_str(),
            "GET /private/api/v1/cogs/fixture/latest HTTP/1.1"
        );
        assert_eq!(
            requests[1].as_str(),
            "GET /private/api/v1/cogs/fixture/1.2.3 HTTP/1.1"
        );
        assert!(
            requests[2..].iter().all(|request| request.as_str()
                == "GET /private/api/v1/security/vulnerabilities/fixture/1.2.3 HTTP/1.1"),
            "unavailable advisories must prevent the download: {requests:?}"
        );
        assert_eq!(std::fs::read(&manifest_path).unwrap(), before);
        assert!(!Manifest::lockfile_path(project.path()).exists());
        assert!(!cache.get_cog_path("fixture", "1.2.3").exists());
        return;
    }
    assert_eq!(
        requests.iter().map(Text::as_str).collect::<List<_>>(),
        List::from(
            &[
                "GET /private/api/v1/cogs/fixture/latest HTTP/1.1",
                "GET /private/api/v1/cogs/fixture/1.2.3 HTTP/1.1",
                "GET /private/api/v1/security/vulnerabilities/fixture/1.2.3 HTTP/1.1",
                "GET /private/api/v1/cogs/fixture/1.2.3/download HTTP/1.1",
            ][..]
        ),
        "metadata and artifact must stay on the selected registry; result: {result:?}",
    );
    if archive_failure {
        assert!(
            result.is_err(),
            "a failed archive transfer must fail installation"
        );
        assert_eq!(std::fs::read(&manifest_path).unwrap(), before);
        assert!(!Manifest::lockfile_path(project.path()).exists());
    } else {
        result.expect("install from selected registry");
        assert_eq!(
            std::fs::read(cache.get_cog_path("fixture", "1.2.3")).unwrap(),
            ARCHIVE
        );
        let lock = Lockfile::from_file(&Manifest::lockfile_path(project.path())).unwrap();
        let entry = lock
            .packages
            .iter()
            .find(|entry| entry.name.as_str() == "fixture")
            .unwrap();
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
