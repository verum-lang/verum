//! T1642: index failures are command failures, before either project file changes.

use super::update;
use crate::config::Manifest;
use crate::registry::Lockfile;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;
use verum_common::Text;

const CHILD: &str = "commands::deps::registry_errors::isolated_registry_case";
const CASE_ENV: &str = "VERUM_T1642_REGISTRY_CASE";

#[test]
fn unavailable_index_never_reports_up_to_date_or_changes_project_files() {
    for case in [
        "first-404",
        "first-503",
        "first-json",
        "first-transport",
        "first-503-absent",
    ] {
        child_case(case);
    }
}

#[test]
fn lockfile_index_failure_preserves_manifest_and_existing_or_absent_lockfile() {
    for case in [
        "second-404",
        "second-503",
        "second-json",
        "second-transport",
        "second-503-absent",
    ] {
        child_case(case);
    }
}

#[test]
fn unresolvable_index_is_not_a_successful_update() {
    for case in [
        "first-empty",
        "first-no-match",
        "second-empty",
        "second-no-match",
    ] {
        child_case(case);
    }
}

#[test]
fn valid_index_preserves_constraints_and_ignores_yanked_versions() {
    child_case("success");
    child_case("up-to-date");
}

fn child_case(case: &str) {
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--nocapture"])
        .env(CASE_ENV, case)
        .output()
        .unwrap();
    let output = format!(
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.status.success(), "{case}: {output}");
    if case != "success" && case != "up-to-date" {
        assert!(
            !output.contains("All dependencies are up to date"),
            "{case}: {output}"
        );
        assert!(
            !output.contains("Updated 1 dependencies"),
            "{case}: {output}"
        );
    }
}

fn request_line(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0 && bytes.len() + count <= 16384);
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string()
}

fn index(versions: &[(&str, bool)]) -> String {
    serde_json::json!({
        "name": "widget",
        "versions": versions.iter().map(|(version, yanked)| serde_json::json!({
            "version": version, "yanked": yanked, "checksum": "a".repeat(64), "features": {},
        })).collect::<Vec<_>>()
    })
    .to_string()
}

#[test]
fn isolated_registry_case() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let project = TempDir::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let configured = format!("{origin}/private");
    // Each case is a fresh process. Accidental public routing reaches this
    // refusal fixture as a proxy; it can never contact a real registry.
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
    let manifest_path = project.path().join(Manifest::MANIFEST_FILENAME);
    std::fs::write(&manifest_path, format!(
        "[cog]\nname = \"consumer\"\nversion = \"1.0.0\"\n[registry]\nindex = \"{configured}\"\n[dependencies]\nwidget = \"^1.0.0\"\n"
    )).unwrap();
    let lock_path = Manifest::lockfile_path(project.path());
    if !case.ends_with("-absent") {
        Lockfile::new("consumer".into())
            .to_file(&lock_path)
            .unwrap();
    }
    let original_manifest = std::fs::read(&manifest_path).unwrap();
    let original_lock = std::fs::read(&lock_path).ok();
    let second = case.starts_with("second-");
    let up_to_date = case == "up-to-date";
    let success = case == "success" || up_to_date;
    let fault = case
        .strip_prefix("first-")
        .or_else(|| case.strip_prefix("second-"))
        .unwrap_or("");
    let fault = fault.strip_suffix("-absent").unwrap_or(fault).to_string();
    let (stop, stopping) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut requests = Vec::new();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let line = request_line(&mut stream);
                    // This test checks selected-origin errors, not readiness
                    // of the still-unimplemented Registry index route.
                    assert!(line.starts_with("GET /private/"), "{line}");
                    requests.push(line);
                    let fail = !success && (!second || requests.len() >= 2);
                    if fail && fault == "transport" {
                        continue;
                    }
                    let (status, body) = if fail {
                        match fault.as_str() {
                            "404" => ("404 Not Found", "missing".to_string()),
                            "503" => ("503 Service Unavailable", "unavailable".to_string()),
                            "json" => ("200 OK", "{invalid".to_string()),
                            "empty" => ("200 OK", index(&[])),
                            "no-match" => ("200 OK", index(&[("2.0.0", false)])),
                            _ => panic!("unexpected fixture {fault}"),
                        }
                    } else if up_to_date {
                        ("200 OK", index(&[("1.0.0", false)]))
                    } else {
                        (
                            "200 OK",
                            index(&[
                                ("1.0.0", false),
                                ("1.1.0", false),
                                ("1.9.0", true),
                                ("2.0.0", false),
                            ]),
                        )
                    };
                    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if stopping.try_recv().is_ok() {
                        return requests;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept: {error}"),
            }
        }
    });
    let outcome = update(None);
    stop.send(()).unwrap();
    let requests = worker.join().unwrap();
    assert_eq!(
        requests.len(),
        if second || case == "success" { 2 } else { 1 },
        "{case}: {requests:?}"
    );
    assert_eq!(outcome.is_ok(), success, "{case}: {outcome:?}");
    if case == "success" {
        let manifest = Manifest::from_file(&manifest_path).unwrap();
        assert!(
            matches!(manifest.dependencies.get(&Text::from("widget")), Some(crate::config::Dependency::Simple(value)) if value.as_str() == "^1.1.0")
        );
        let lock = Lockfile::from_file(&lock_path).unwrap();
        assert_eq!(lock.get_cog("widget").unwrap().version.as_str(), "1.1.0");
    } else {
        assert_eq!(
            std::fs::read(&manifest_path).unwrap(),
            original_manifest,
            "{case}"
        );
        assert_eq!(std::fs::read(&lock_path).ok(), original_lock, "{case}");
    }
}
