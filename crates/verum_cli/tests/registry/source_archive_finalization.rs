//! T1742: distinguish successful tar termination from failed gzip completion.
//! These are writer-error and byte-preservation controls, not durable storage.

use super::finish_source_archive;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use std::cell::RefCell;
use std::io::{self, Write};
use std::rc::Rc;
use verum_common::{List, Maybe};

const CONTENT: &[u8] = b"\x00\xff\x42\x80\x7f\xfe\x00";
const WRITE_ERROR: &str = "fixture finalization write refused";
const FLUSH_ERROR: &str = "fixture final output flush refused";

#[derive(Default)]
struct WriterState {
    bytes: List<u8>,
    fail_after: Maybe<usize>,
    fail_flush: bool,
    write_failures: usize,
    flushes: usize,
}

#[derive(Clone, Default)]
struct RecordingWriter(Rc<RefCell<WriterState>>);

impl RecordingWriter {
    fn bytes(&self) -> List<u8> {
        self.0.borrow().bytes.clone()
    }
}

impl Write for RecordingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let mut state = self.0.borrow_mut();
        let count = state
            .fail_after
            .map(|limit| limit.saturating_sub(state.bytes.len()).min(bytes.len()))
            .unwrap_or(bytes.len());
        if count == 0 {
            state.write_failures += 1;
            return Err(io::Error::other(WRITE_ERROR));
        }
        state.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut state = self.0.borrow_mut();
        state.flushes += 1;
        if state.fail_flush {
            Err(io::Error::other(FLUSH_ERROR))
        } else {
            Ok(())
        }
    }
}

fn archive(
    writer: RecordingWriter,
    level: Compression,
) -> tar::Builder<GzEncoder<RecordingWriter>> {
    let mut archive = tar::Builder::new(GzEncoder::new(writer, level));
    let mut header = tar::Header::new_gnu();
    header.set_size(CONTENT.len() as u64);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    archive
        .append_data(&mut header, "src/data.bin", CONTENT)
        .unwrap();
    archive
}

fn legacy_bytes(level: Compression) -> List<u8> {
    let writer = RecordingWriter::default();
    let mut archive = archive(writer.clone(), level);
    archive.finish().unwrap();
    drop(archive); // The old producer completes gzip only through Drop.
    writer.bytes()
}

fn trailer_failure(level: Compression) -> (RecordingWriter, List<u8>, usize) {
    let expected = legacy_bytes(level);
    // Refuse the fourth byte of the eight-byte CRC32/ISIZE trailer. This
    // position is independent of compression buffering and Write chunk sizes.
    let limit = expected.len().checked_sub(5).unwrap();
    let writer = RecordingWriter::default();
    writer.0.borrow_mut().fail_after = Some(limit);
    (writer, expected, limit)
}

fn assert_truncated_trailer(writer: &RecordingWriter, expected: &[u8], limit: usize) {
    let bytes = writer.bytes();
    assert_eq!(bytes.len(), limit);
    assert_eq!(bytes.as_slice(), &expected[..limit]);
    assert!(writer.0.borrow().write_failures > 0);
    let mut decoded = List::new();
    assert!(io::copy(&mut GzDecoder::new(bytes.as_slice()), &mut decoded).is_err());
}

#[test]
fn drop_only_path_hides_gzip_trailer_failure_after_successful_tar_termination() {
    for level in [Compression::default(), Compression::best()] {
        let (writer, expected, limit) = trailer_failure(level);
        let mut archive = archive(writer.clone(), level);
        let reported = archive.finish();
        assert!(
            reported.is_ok(),
            "tar termination must already have succeeded"
        );
        assert_eq!(writer.0.borrow().write_failures, 0);
        assert!(writer.bytes().len() <= expected.len() - 8);
        drop(archive);
        assert!(
            reported.is_ok(),
            "the legacy result never observes Drop errors"
        );
        assert_truncated_trailer(&writer, expected.as_slice(), limit);
    }
}

#[test]
fn source_archive_reports_gzip_trailer_failure_after_successful_tar_termination() {
    for level in [Compression::default(), Compression::best()] {
        let (writer, expected, limit) = trailer_failure(level);
        let mut archive = archive(writer.clone(), level);
        archive
            .finish()
            .expect("tar end blocks accepted before gzip finalization");
        assert_eq!(writer.0.borrow().write_failures, 0);
        assert!(writer.bytes().len() <= expected.len() - 8);
        let result = finish_source_archive(archive);
        assert_truncated_trailer(&writer, expected.as_slice(), limit);
        let error = result.expect_err("gzip trailer failure must reach the producer");
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), WRITE_ERROR);
    }
}

#[test]
fn source_archive_reports_tar_termination_write_failure() {
    let writer = RecordingWriter::default();
    writer.0.borrow_mut().fail_after = Some(0);
    // An empty tar has no entry writes: its first output is triggered by
    // writing the end blocks, before finishing the inner gzip stream.
    let archive = tar::Builder::new(GzEncoder::new(writer.clone(), Compression::best()));
    let error = finish_source_archive(archive).unwrap_err();
    assert_eq!(error.to_string(), WRITE_ERROR);
    assert!(writer.bytes().is_empty());
    assert!(writer.0.borrow().write_failures > 0);
}

#[test]
fn source_archive_reports_final_output_flush_failure() {
    let writer = RecordingWriter::default();
    writer.0.borrow_mut().fail_flush = true;
    let result = finish_source_archive(archive(writer.clone(), Compression::best()));
    assert_eq!(writer.bytes(), legacy_bytes(Compression::best()));
    let error = result.expect_err("a complete stream does not erase a final flush error");
    assert_eq!(error.to_string(), FLUSH_ERROR);
    assert_eq!(writer.0.borrow().flushes, 1);
}

#[test]
fn source_archive_preserves_normal_bytes_and_binary_tar_content() {
    for level in [Compression::default(), Compression::best()] {
        let writer = RecordingWriter::default();
        finish_source_archive(archive(writer.clone(), level)).unwrap();
        let bytes = writer.bytes();
        assert_eq!(bytes, legacy_bytes(level));
        let mut tar_bytes = List::new();
        io::copy(&mut GzDecoder::new(bytes.as_slice()), &mut tar_bytes).unwrap();
        assert_eq!(tar_bytes.len() % 512, 0);
        assert!(
            tar_bytes.as_slice()[tar_bytes.len() - 1024..]
                .iter()
                .all(|byte| *byte == 0)
        );
        let mut tar = tar::Archive::new(tar_bytes.as_slice());
        let mut entries = tar.entries().unwrap();
        let mut entry = entries.next().unwrap().unwrap();
        assert_eq!(
            entry.path().unwrap().as_ref(),
            std::path::Path::new("src/data.bin")
        );
        let mut content = List::new();
        io::copy(&mut entry, &mut content).unwrap();
        assert_eq!(content.as_slice(), CONTENT);
        assert!(entries.next().is_none());
    }
}
