//! T1636: publication must transmit metadata as well as archive bytes.
//! This loopback capture is a transport regression, not registry acceptance.

use super::RegistryClient;
use crate::registry::CogMetadata;
use reqwest::blocking::Client;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use verum_common::{List, Map, Text};

const ARCHIVE: &[u8] = b"\x1f\x8bfixture-archive\0\xff\r\nHTTP/1.1\r\n";
const DESCRIPTION: &str = "metadata-must-survive-publication";

#[test]
fn publish_transmits_metadata_alongside_archive() {
    let project = TempDir::new().expect("temporary package");
    let archive_path = project.path().join("fixture-1.2.3.vr");
    std::fs::write(&archive_path, ARCHIVE).unwrap();
    let metadata: CogMetadata = serde_json::from_value(serde_json::json!({
        "name": "fixture", "version": "1.2.3", "description": DESCRIPTION,
        "authors": [], "license": null, "repository": null, "homepage": null,
        "keywords": [], "categories": [], "readme": null,
        "dependencies": {}, "features": {}, "artifacts": {},
        "proofs": null, "cbgr_profiles": null, "signature": null,
        "ipfs_hash": null, "checksum": hex::encode(Sha256::digest(ARCHIVE)),
        "published_at": 1
    }))
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    listener.set_nonblocking(true).unwrap();
    let base_url: Text = format!("http://{}/private", listener.local_addr().unwrap()).into();
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "client did not connect");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture accept failed: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let request = capture_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        request
    });
    // Keep this controlled capture independent of the machine's proxy settings.
    let client = RegistryClient {
        base_url,
        client: Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
    };
    let result = client.publish(&metadata, &archive_path, "fixture-publish-token");
    let request = worker.join().expect("request capture");
    result.expect("fixture HTTP response");
    assert_eq!(
        request.request_line.as_str(),
        "POST /private/api/v1/cogs/publish HTTP/1.1"
    );
    assert_eq!(
        request
            .headers
            .get(&Text::from("authorization"))
            .map(Text::as_str),
        Some("Bearer fixture-publish-token"),
    );
    assert!(
        request
            .body
            .windows(DESCRIPTION.len())
            .any(|part| part == DESCRIPTION.as_bytes()),
        "publication discarded metadata: content-type={:?}, body_is_exact_archive={}, archive_bytes={}",
        request.headers.get(&Text::from("content-type")),
        request.body.as_slice() == ARCHIVE,
        request.body.len(),
    );
}

struct CapturedRequest {
    request_line: Text,
    headers: Map<Text, Text>,
    body: List<u8>,
}

fn capture_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut bytes: List<u8> = List::new();
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
        let count = stream.read(&mut buffer).expect("request header read");
        assert!(count > 0, "request ended before its headers");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 16 * 1024, "fixture header bound");
    };
    let header =
        std::str::from_utf8(&bytes.as_slice()[..header_end]).expect("ASCII request headers");
    let mut lines = header.lines();
    let request_line = lines.next().unwrap().into();
    let mut headers = Map::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').expect("header separator");
        headers.insert(
            Text::from(name.to_ascii_lowercase()),
            Text::from(value.trim()),
        );
    }
    let body_len: usize = headers
        .get(&Text::from("content-length"))
        .expect("bounded request has content length")
        .as_str()
        .parse()
        .unwrap();
    assert!(body_len <= 64 * 1024, "fixture body bound");
    while bytes.len() - header_end < body_len {
        let count = stream.read(&mut buffer).expect("request body read");
        assert!(count > 0, "request body truncated");
        bytes.extend_from_slice(&buffer[..count]);
    }
    assert_eq!(bytes.len() - header_end, body_len, "one complete request");
    CapturedRequest {
        request_line,
        headers,
        body: bytes.as_slice()[header_end..].iter().copied().collect(),
    }
}
