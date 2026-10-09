//! T1636: a successful HTTP status alone must never announce publication.

use super::publication_transport::{metadata, publish_to_fixture, publish_to_response};
use verum_common::Text;

fn receipt() -> Text {
    let metadata = metadata();
    serde_json::to_string(&serde_json::json!({
        "name": metadata.name, "version": metadata.version, "checksum": metadata.checksum,
    }))
    .unwrap()
    .into()
}

#[test]
fn publication_accepts_only_matching_created_or_retry_receipts() {
    let receipt = receipt();
    for status in ["201 Created", "200 OK"] {
        for media in ["application/json", "application/json; charset=\"UTF-8\""] {
            let header = format!("Content-Type: {media}\r\n");
            let (result, _) = publish_to_fixture(&metadata(), status, receipt.as_bytes(), &header);
            result.expect("matching coordinate/digest receipt");
        }
    }
    for status in ["202 Accepted", "204 No Content"] {
        let (result, _) = publish_to_fixture(
            &metadata(),
            status,
            receipt.as_bytes(),
            "Content-Type: application/json\r\n",
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no publication receipt")
        );
    }
}

#[test]
fn publication_rejects_malformed_duplicate_and_authority_receipts() {
    let valid = receipt();
    let checksum = metadata().checksum;
    for body in [
        Text::from(""),
        Text::from("not JSON"),
        Text::from("{}"),
        Text::from("[]"),
        format!("{valid} {valid}").into(),
        format!("{{\"name\":\"fixture\",\"name\":\"fixture\",\"version\":\"1.2.3\",\"checksum\":\"{checksum}\"}}").into(),
        format!("{{\"name\":\"fixture\",\"\\u006eame\":\"fixture\",\"version\":\"1.2.3\",\"checksum\":\"{checksum}\"}}").into(),
        format!("{{\"name\":\"fixture\",\"version\":\"1.2.3\",\"checksum\":\"{checksum}\",\"authority\":\"claimed-owner\"}}").into(),
    ] {
        let (result, _) = publish_to_fixture(&metadata(), "201 Created", body.as_bytes(), "Content-Type: application/json\r\n");
        let error = result.unwrap_err();
        assert!(error.to_string().contains("Invalid publication receipt"), "{body:?}: {error}");
    }
}

#[test]
fn publication_rejects_coordinate_or_checksum_mismatch() {
    for (field, replacement) in [
        ("name", "other"),
        ("version", "2.0.0"),
        (
            "checksum",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ),
    ] {
        let mut value: serde_json::Value = serde_json::from_str(receipt().as_str()).unwrap();
        value[field] = replacement.into();
        let body = serde_json::to_string(&value).unwrap();
        let (result, _) = publish_to_fixture(
            &metadata(),
            "200 OK",
            body.as_bytes(),
            "Content-Type: application/json\r\n",
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("does not match requested name, version and archive checksum")
        );
    }
}

#[test]
fn publication_receipt_requires_json_and_utf8_media_type() {
    let body = receipt();
    for header in [
        "",
        "Content-Type: text/plain\r\n",
        "Content-Type: application/json; charset=latin1\r\n",
        "Content-Type: application/json; charset=\"utf-8\r\n",
        "Content-Type: application/json; extra=value\r\n",
    ] {
        let (result, _) = publish_to_fixture(&metadata(), "201 Created", body.as_bytes(), header);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("expected application/json")
        );
    }
}

#[test]
fn publication_receipt_bound_covers_declared_and_streamed_bodies() {
    let receipt = receipt();
    let at_limit: Text = format!("{receipt}{}", " ".repeat(65536 - receipt.len())).into();
    let (accepted, _) = publish_to_fixture(
        &metadata(),
        "201 Created",
        at_limit.as_bytes(),
        "Content-Type: application/json\r\n",
    );
    accepted.expect("the exact receipt boundary is accepted");
    let too_large: Text = format!("{at_limit} ").into();
    let (declared, _) = publish_to_fixture(
        &metadata(),
        "201 Created",
        too_large.as_bytes(),
        "Content-Type: application/json\r\n",
    );
    assert!(
        declared
            .unwrap_err()
            .to_string()
            .contains("exceeds 65536-byte limit")
    );
    let (streamed, _) = publish_to_response(
        &metadata(),
        "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
        too_large.as_bytes(),
    );
    assert!(
        streamed
            .unwrap_err()
            .to_string()
            .contains("exceeds 65536-byte limit")
    );
}
