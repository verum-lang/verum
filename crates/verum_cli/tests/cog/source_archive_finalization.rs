//! T1742: exercise the actual source-publication archive producer without HTTP.
//! Optional retained bytes are inputs for independent shared-reader gates.

use super::create_cog_tarball;
use crate::config::Manifest;
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::PathBuf;
use tempfile::TempDir;
use verum_common::{List, Map, Text};

const RETAIN_ENV: &str = "VERUM_T1742_ARCHIVE_FIXTURE_DIR";

struct RemoveArchive(PathBuf);

impl Drop for RemoveArchive {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn sha256(bytes: &[u8]) -> Text {
    hex::encode(Sha256::digest(bytes)).into()
}

fn retain_fixture(compressed: &[u8], tar: &[u8], expected: &Map<Text, List<u8>>) {
    let Some(root) = std::env::var_os(RETAIN_ENV) else {
        return;
    };
    let root = PathBuf::from(root);
    assert!(root.is_absolute(), "{RETAIN_ENV} must be an absolute path");
    fs::create_dir_all(&root).unwrap();
    let destination = root.join("source-cog");
    fs::create_dir(&destination)
        .expect("retain to a new directory; never replace earlier evidence");
    fs::write(destination.join("archive.vr"), compressed).unwrap();
    fs::write(destination.join("archive.tar"), tar).unwrap();
    let mut entries: List<_> = expected.iter().collect();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    let mut entry_receipts: List<serde_json::Value> = List::new();
    for (path, content) in entries {
        let input = destination.join("input").join(path.as_str());
        fs::create_dir_all(input.parent().unwrap()).unwrap();
        fs::write(&input, content.as_slice()).unwrap();
        entry_receipts.push(serde_json::json!({
            "path": path.as_str(),
            "size": content.len(),
            "sha256": sha256(content.as_slice()),
        }));
    }
    let receipt = serde_json::json!({
        "producer": "verum_cli::cog::create_cog_tarball",
        "profile": "source files with GNU long-name extension",
        "compressed": {
            "path": "archive.vr", "size": compressed.len(), "sha256": sha256(compressed),
        },
        "tar": { "path": "archive.tar", "size": tar.len(), "sha256": sha256(tar) },
        "entries": entry_receipts,
        "scope": "actual CLI producer bytes; shared Verum decoding remains a separate gate",
    });
    let json: Text = serde_json::to_string_pretty(&receipt).unwrap().into();
    fs::write(destination.join("expected.json"), json.as_bytes()).unwrap();
    eprintln!(
        "retained actual source-cog fixture at {}",
        destination.display()
    );
}

#[test]
fn actual_source_cog_preserves_short_long_and_binary_entries() {
    let project = TempDir::new().unwrap();
    let name: Text = format!("archive-finalization-{}", uuid::Uuid::new_v4().simple()).into();
    let manifest_source: Text = format!("[cog]\nname = \"{name}\"\nversion = \"1.2.3\"\n").into();
    let long_path: Text = format!("src/{}/{}/binary.dat", "a".repeat(64), "b".repeat(64)).into();
    assert!(
        long_path.len() > 100,
        "exercise the real producer's GNU long-name path"
    );
    let mut expected: Map<Text, List<u8>> = Map::new();
    expected.insert(
        Manifest::MANIFEST_FILENAME.into(),
        List::from(manifest_source.as_bytes()),
    );
    expected.insert(
        "src/short.bin".into(),
        List::from(&b"short\x00\xff\x80"[..]),
    );
    expected.insert(
        long_path.clone(),
        List::from(&b"\x00\xff\x42\x80\x7f\xfe\x00"[..]),
    );
    for (path, content) in &expected {
        let input = project.path().join(path.as_str());
        fs::create_dir_all(input.parent().unwrap()).unwrap();
        fs::write(input, content.as_slice()).unwrap();
    }
    let manifest = Manifest::from_file(&project.path().join(Manifest::MANIFEST_FILENAME)).unwrap();
    // Register cleanup before invoking the producer so even a partial output
    // from a failed assertion or packaging error is removed from the temp root.
    let cleanup = RemoveArchive(std::env::temp_dir().join(format!("{name}-1.2.3.vr")));
    let archive_path = create_cog_tarball(project.path(), &manifest).unwrap();
    assert_eq!(archive_path, cleanup.0);
    let compressed: List<u8> = fs::read(&archive_path).unwrap().into();
    let mut tar_bytes: List<u8> = List::new();
    io::copy(&mut GzDecoder::new(compressed.as_slice()), &mut tar_bytes).unwrap();

    let mut archive = tar::Archive::new(tar_bytes.as_slice());
    let mut actual: Map<Text, List<u8>> = Map::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        assert!(entry.header().entry_type().is_file());
        let path: Text = entry.path().unwrap().to_str().unwrap().into();
        let mut content = List::new();
        io::copy(&mut entry, &mut content).unwrap();
        assert!(
            actual.insert(path, content).is_none(),
            "duplicate archive entry"
        );
    }
    assert_eq!(actual, expected);
    assert!(actual.contains_key(&long_path));

    let mut raw = tar::Archive::new(tar_bytes.as_slice());
    let long_names = raw
        .entries()
        .unwrap()
        .raw(true)
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .header()
                .entry_type()
                .is_gnu_longname()
        })
        .count();
    assert_eq!(
        long_names, 1,
        "retain an actual GNU extension, not a handcrafted surrogate"
    );
    assert_eq!(tar_bytes.len() % 512, 0);
    assert!(
        tar_bytes.as_slice()[tar_bytes.len() - 1024..]
            .iter()
            .all(|byte| *byte == 0)
    );
    retain_fixture(compressed.as_slice(), tar_bytes.as_slice(), &expected);
}
