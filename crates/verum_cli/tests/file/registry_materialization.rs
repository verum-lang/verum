//! Actual script dependency resolver and source materializer against a loopback
//! metadata/archive fixture. These tests do not establish Registry application acceptance.
use super::resolve_registry_dep_with;
use crate::registry::{CacheManager, RegistryClient};
use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::{fs, thread, time::Duration};

const SOURCE: &[u8] = b"module widget.api;\npublic fn value() -> Int { 42 }\n";

struct ResponseState {
    archive: Vec<u8>,
    name: String,
    latest: String,
    requests: Vec<String>,
}

struct RegistryFixture {
    url: String,
    state: Arc<Mutex<ResponseState>>,
    stopped: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RegistryFixture {
    fn new(archive: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(ResponseState {
            archive,
            name: "widget".into(),
            latest: "1.2.3".into(),
            requests: Vec::new(),
        }));
        let stopped = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&state);
        let stop = Arc::clone(&stopped);
        let thread = thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let (mut socket, _) = match listener.accept() {
                    Ok(socket) => socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("fixture listener: {error}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    if socket.read(&mut byte).unwrap() == 0 {
                        break;
                    }
                    request.push(byte[0]);
                    assert!(request.len() < 16384);
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_owned();
                let mut state = shared.lock().unwrap();
                state.requests.push(path.clone());
                let (status, body) = match path.as_str() {
                    "/api/v1/cogs/widget/latest" => (
                        200,
                        serde_json::to_vec(&serde_json::json!({"version":state.latest})).unwrap(),
                    ),
                    "/api/v1/cogs/widget/1.2.3" => (
                        200,
                        serde_json::to_vec(&serde_json::json!({
                            "name":state.name,"version":"1.2.3","authors":[],"keywords":[],
                            "categories":[],"dependencies":{},"features":{},"artifacts":{},
                            "checksum":hex::encode(Sha256::digest(&state.archive)),"published_at":1,
                        }))
                        .unwrap(),
                    ),
                    "/api/v1/cogs/widget/1.2.3/download" => (200, state.archive.clone()),
                    _ => (404, Vec::new()),
                };
                drop(state);
                write!(socket, "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                socket.write_all(&body).unwrap();
            }
        });
        Self {
            url,
            state,
            stopped,
            thread: Some(thread),
        }
    }

    fn client(&self) -> RegistryClient {
        RegistryClient::new(self.url.clone()).unwrap()
    }
}

impl Drop for RegistryFixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn archive(name: &str, duplicate: bool) -> Vec<u8> {
    let manifest = format!("[cog]\nname={name:?}\nversion=\"1.2.3\"\nedition=\"2026\"\n");
    let mut members = vec![("Verum.toml", manifest.as_bytes()), ("src/api.vr", SOURCE)];
    if duplicate {
        members.push(("src/api.vr", b"replacement"));
    }
    let mut tar = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for (name, bytes) in members {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        tar.append_data(&mut header, name, bytes).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}

#[test]
fn partial_shared_cache_never_counts_as_installed_and_retry_recovers() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let partial = cache.join("widget/1.2.3");
    fs::create_dir_all(partial.join("src")).unwrap();
    fs::write(partial.join("verum.toml"), b"old incomplete manifest").unwrap();
    fs::write(partial.join("src/api.vr"), b"old incomplete source").unwrap();
    let server = RegistryFixture::new(archive("widget", true));
    for _ in 0..2 {
        assert!(
            resolve_registry_dep_with("widget", Some("1.2.3"), &server.client(), &cache).is_err()
        );
    }
    server.state.lock().unwrap().archive = archive("widget", false);
    let entry =
        resolve_registry_dep_with("widget", Some("1.2.3"), &server.client(), &cache).unwrap();
    assert_ne!(entry.root, partial);
    assert_eq!(fs::read(entry.root.join("src/api.vr")).unwrap(), SOURCE);
    assert_eq!(
        fs::read(partial.join("src/api.vr")).unwrap(),
        b"old incomplete source"
    );
    let archive = CacheManager::new(cache)
        .unwrap()
        .get_cog_path("widget", "1.2.3");
    let digest = hex::encode(Sha256::digest(fs::read(archive).unwrap()));
    assert_eq!(entry.locked.integrity, format!("sha256-source-{digest}"));
    let root = entry.root.clone();
    drop(entry);
    assert!(
        !root.exists(),
        "the owning result must release its private snapshot"
    );
}

#[test]
fn concurrent_resolves_own_independent_complete_snapshots() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let manager = CacheManager::new(cache.clone()).unwrap();
    let path = manager.get_cog_path("widget", "1.2.3");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes = archive("widget", false);
    fs::write(path, &bytes).unwrap();
    let server = RegistryFixture::new(bytes);
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let cache = cache.clone();
            let url = server.url.clone();
            thread::spawn(move || {
                let entry = resolve_registry_dep_with(
                    "widget",
                    Some("1.2.3"),
                    &RegistryClient::new(url).unwrap(),
                    &cache,
                )
                .unwrap();
                assert_eq!(fs::read(entry.root.join("src/api.vr")).unwrap(), SOURCE);
                entry
            })
        })
        .collect();
    let entries: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    let roots: std::collections::HashSet<_> =
        entries.iter().map(|entry| entry.root.clone()).collect();
    assert_eq!(roots.len(), 4);
    assert!(roots.iter().all(|root| root.is_dir()));
    drop(entries);
    assert!(roots.iter().all(|root| !root.exists()));
}

#[test]
fn source_snapshot_rejects_manifest_mismatch_and_corrupt_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let server = RegistryFixture::new(archive("other", false));
    assert!(
        resolve_registry_dep_with("widget", Some("1.2.3"), &server.client(), tmp.path()).is_err()
    );
    server.state.lock().unwrap().archive = archive("widget", false);
    let entry =
        resolve_registry_dep_with("widget", Some("1.2.3"), &server.client(), tmp.path()).unwrap();
    let manager = CacheManager::new(tmp.path().to_owned()).unwrap();
    let checksum = entry.source.as_ref().unwrap().checksum().to_owned();
    fs::write(manager.get_cog_path("widget", "1.2.3"), b"corrupt archive").unwrap();
    assert!(
        manager
            .materialize_source("widget", "1.2.3", &checksum)
            .is_err()
    );
    assert_eq!(fs::read(entry.root.join("src/api.vr")).unwrap(), SOURCE);
    let repaired =
        resolve_registry_dep_with("widget", Some("1.2.3"), &server.client(), tmp.path()).unwrap();
    assert_eq!(repaired.locked.integrity, entry.locked.integrity);
}

#[cfg(unix)]
#[test]
fn linked_cached_archive_cannot_be_materialized() {
    let tmp = tempfile::tempdir().unwrap();
    let server = RegistryFixture::new(archive("widget", false));
    let manager = CacheManager::new(tmp.path().join("cache")).unwrap();
    let cached = manager.get_cog_path("widget", "1.2.3");
    fs::create_dir_all(cached.parent().unwrap()).unwrap();
    let external = tmp.path().join("external.gz");
    let bytes = server.state.lock().unwrap().archive.clone();
    fs::write(&external, &bytes).unwrap();
    std::os::unix::fs::symlink(&external, &cached).unwrap();
    assert!(
        resolve_registry_dep_with(
            "widget",
            Some("1.2.3"),
            &server.client(),
            &tmp.path().join("cache")
        )
        .is_err()
    );
    assert_eq!(fs::read_link(cached).unwrap(), external);
    assert_eq!(fs::read(external).unwrap(), bytes);
}

#[test]
fn remote_coordinates_and_version_constraints_are_checked_before_download() {
    let tmp = tempfile::tempdir().unwrap();
    let server = RegistryFixture::new(archive("widget", false));
    server.state.lock().unwrap().name = "other".into();
    assert!(
        resolve_registry_dep_with("widget", Some("1.2.3"), &server.client(), tmp.path()).is_err()
    );
    server.state.lock().unwrap().latest = "2.0.0".into();
    assert!(resolve_registry_dep_with("widget", Some("^1"), &server.client(), tmp.path()).is_err());
    server.state.lock().unwrap().latest = "../../outside".into();
    assert!(resolve_registry_dep_with("widget", None, &server.client(), tmp.path()).is_err());
    assert!(resolve_registry_dep_with("../outside", None, &server.client(), tmp.path()).is_err());
    assert!(
        !server
            .state
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|path| path.ends_with("/download"))
    );
}
