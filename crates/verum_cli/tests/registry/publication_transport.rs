//! T1636: publication must transmit metadata as well as archive bytes.
//! These loopback captures are transport regressions, not registry acceptance.

use super::{RegistryClient, publication_client_builder};
use crate::error::Result;
use crate::registry::publication::{MAX_ARCHIVE_BYTES, MAX_METADATA_BYTES, PublicationLimits};
use crate::registry::{CogMetadata, CogSignature};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use verum_common::{List, Map, Text};

// Deliberately opaque binary bytes: archive validity is a separate server gate.
const ARCHIVE: &[u8] = b"\x1f\x8bfixture-archive\0\xff\r\nHTTP/1.1\r\n";
const DESCRIPTION: &str = "metadata-must-survive-publication-\u{03bb}";

#[test]
fn publish_transmits_metadata_alongside_archive() {
    let (result, request) = publish_to_fixture(&metadata(), "201 Created", &[], "");
    result.expect("fixture HTTP response");
    assert_eq!(
        request.request_line.as_str(),
        "POST /private/api/v1/cogs/publish HTTP/1.1"
    );
    assert_eq!(
        header(&request, "authorization"),
        Some("Bearer fixture-publish-token")
    );
    assert_eq!(
        header(&request, "content-type"),
        Some("application/vnd.verum.cog-publish.v1")
    );
    assert!(
        request
            .body
            .windows(DESCRIPTION.len())
            .any(|part| part == DESCRIPTION.as_bytes()),
        "publication discarded metadata: content-type={:?}, body_is_exact_archive={}, archive_bytes={}",
        header(&request, "content-type"),
        request.body.as_slice() == ARCHIVE,
        request.body.len(),
    );
    // Decode the public contract independently of the production encoder.
    let bytes = request.body.as_slice();
    let json_len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
    let json: serde_json::Value = serde_json::from_slice(&bytes[4..4 + json_len]).unwrap();
    let archive_len = u64::from_le_bytes(bytes[4 + json_len..12 + json_len].try_into().unwrap());
    assert_eq!(archive_len, ARCHIVE.len() as u64);
    assert_eq!(
        &bytes[12 + json_len..],
        ARCHIVE,
        "exact binary archive and no trailing bytes"
    );
    assert_eq!(json["name"], "fixture");
    assert_eq!(json["version"], "1.2.3");
    assert_eq!(json["description"], DESCRIPTION);
    assert_eq!(json["checksum"], hex::encode(Sha256::digest(ARCHIVE)));
    let object = json.as_object().unwrap();
    assert_eq!(object.len(), 13, "the source-only field set is fixed");
    for forbidden in [
        "published_at",
        "authority",
        "as_authority",
        "proofs",
        "signature",
        "artifacts",
        "cbgr_profiles",
        "ipfs_hash",
    ] {
        assert!(
            !object.contains_key(forbidden),
            "server-owned or unsupported field {forbidden}"
        );
    }
}

#[test]
fn invalid_metadata_and_changed_archive_fail_before_http() {
    for (field, value, reason) in [
        (
            "name",
            "../fixture",
            "Invalid publication metadata: cog name",
        ),
        ("version", "latest", "Invalid publication metadata: version"),
        (
            "checksum",
            "not-a-digest",
            "Invalid publication metadata: checksum",
        ),
        (
            "checksum",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "Publication archive checksum mismatch",
        ),
    ] {
        let mut metadata = metadata();
        match field {
            "name" => metadata.name = value.into(),
            "version" => metadata.version = value.into(),
            _ => metadata.checksum = value.into(),
        }
        assert_rejected_without_http(&metadata, ARCHIVE, PublicationLimits::default(), reason);
    }
    assert_rejected_without_http(
        &metadata(),
        b"",
        PublicationLimits::default(),
        "Publication archive is empty",
    );
}

#[test]
fn evidence_claims_are_refused_instead_of_silently_discarded() {
    let mut signed = metadata();
    signed.signature = Some(CogSignature {
        public_key: "publisher-key".into(),
        signature: "publisher-signature".into(),
        signed_at: 1,
    });
    assert_rejected_without_http(
        &signed,
        ARCHIVE,
        PublicationLimits::default(),
        "source metadata only",
    );
    for field in ["proofs", "cbgr_profiles", "artifacts", "ipfs_hash"] {
        let mut value = serde_json::to_value(metadata()).unwrap();
        value[field] = match field {
            "proofs" => {
                serde_json::json!({"solver":"claimed-solver", "proofs":[], "level":"proof"})
            }
            "cbgr_profiles" => {
                serde_json::json!({"default":{"avg_check_ns":0.0,"memory_overhead_pct":0.0,"optimizable_refs":1,"total_checks":1},"optimized":null,"minimal":null})
            }
            "artifacts" => {
                serde_json::json!({"tier1":{"path":"claimed-build","checksum":"x","size":1,"target":null}})
            }
            _ => serde_json::json!("claimed-ipfs-content"),
        };
        let claims: CogMetadata = serde_json::from_value(value).unwrap();
        assert_rejected_without_http(
            &claims,
            ARCHIVE,
            PublicationLimits::default(),
            "source metadata only",
        );
    }
}

#[test]
fn configured_limits_are_validated_and_enforced_before_http() {
    for (metadata_bytes, archive_bytes) in [
        (0, 1),
        (1, 0),
        (MAX_METADATA_BYTES + 1, 1),
        (1, MAX_ARCHIVE_BYTES + 1),
        (usize::MAX, usize::MAX),
    ] {
        assert!(PublicationLimits::new(metadata_bytes, archive_bytes).is_err());
    }
    assert_rejected_without_http(
        &metadata(),
        ARCHIVE,
        PublicationLimits::new(1, 1024).unwrap(),
        "metadata exceeds configured limit",
    );
    assert_rejected_without_http(
        &metadata(),
        ARCHIVE,
        PublicationLimits::new(1024, ARCHIVE.len() - 1).unwrap(),
        "archive exceeds configured limit",
    );
}

#[test]
fn publication_does_not_follow_redirects() {
    let destination = TcpListener::bind("127.0.0.1:0").unwrap();
    destination.set_nonblocking(true).unwrap();
    let location = format!(
        "Location: http://{}/stolen\r\n",
        destination.local_addr().unwrap()
    );
    let (result, _) = publish_to_fixture(&metadata(), "307 Temporary Redirect", &[], &location);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Publish failed: 307")
    );
    assert_eq!(
        destination.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

fn metadata() -> CogMetadata {
    serde_json::from_value(serde_json::json!({
        "name": "fixture", "version": "1.2.3", "description": DESCRIPTION,
        "authors": [], "license": null, "repository": null, "homepage": null,
        "keywords": [], "categories": [], "readme": null,
        "dependencies": {}, "features": {}, "artifacts": {},
        "proofs": null, "cbgr_profiles": null, "signature": null,
        "ipfs_hash": null, "checksum": hex::encode(Sha256::digest(ARCHIVE)),
        "published_at": 1
    }))
    .unwrap()
}

fn client(base_url: Text) -> RegistryClient {
    // Preserve the production redirect policy while isolating ambient proxies.
    let transport = publication_client_builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    RegistryClient {
        base_url,
        client: transport.clone(),
        publication_client: Some(transport),
        publication_limits: PublicationLimits::default(),
    }
}

fn assert_rejected_without_http(
    metadata: &CogMetadata,
    bytes: &[u8],
    limits: PublicationLimits,
    reason: &str,
) {
    let project = TempDir::new().unwrap();
    let archive = project.path().join("fixture.vr");
    std::fs::write(&archive, bytes).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let client = client(format!("http://{}", listener.local_addr().unwrap()).into())
        .with_publication_limits(limits);
    let error = client
        .publish(metadata, &archive, "fixture-publish-token")
        .unwrap_err();
    assert!(
        error.to_string().contains(reason),
        "expected {reason:?}, got {error}"
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

fn publish_to_fixture(
    metadata: &CogMetadata,
    status: &str,
    response_body: &[u8],
    extra_headers: &str,
) -> (Result<()>, CapturedRequest) {
    let project = TempDir::new().expect("temporary package");
    let archive = project.path().join("fixture-1.2.3.vr");
    std::fs::write(&archive, ARCHIVE).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    listener.set_nonblocking(true).unwrap();
    let base_url: Text = format!("http://{}/private", listener.local_addr().unwrap()).into();
    let response_header: Text = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n",
        response_body.len()
    )
    .into();
    let response_body: List<u8> = response_body.iter().copied().collect();
    // TLS/proxy initialization is outside the fixture's connection deadline.
    let client = client(base_url)
        .with_publication_limits(PublicationLimits::new(1024, ARCHIVE.len()).unwrap());
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
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let request = capture_request(&mut stream);
        stream.write_all(response_header.as_bytes()).unwrap();
        stream.write_all(response_body.as_slice()).unwrap();
        request
    });
    let result = client.publish(metadata, &archive, "fixture-publish-token");
    (result, worker.join().expect("request capture"))
}

struct CapturedRequest {
    request_line: Text,
    headers: Map<Text, Text>,
    body: List<u8>,
}

fn header<'a>(request: &'a CapturedRequest, name: &str) -> Option<&'a str> {
    request.headers.get(&Text::from(name)).map(Text::as_str)
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
