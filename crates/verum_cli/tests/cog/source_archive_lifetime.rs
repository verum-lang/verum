//! Each publication owns complete source bytes until its upload finishes.

use super::{add_directory_to_archive, create_cog_tarball};
use crate::config::Manifest;
use std::fs;
use std::path::Path;
use tempfile::TempDir;
use verum_common::List;

fn project(name: &str, source: &[u8]) -> TempDir {
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join(Manifest::MANIFEST_FILENAME),
        format!("[cog]\nname = \"{name}\"\nversion = \"1.2.3\"\n"),
    )
    .unwrap();
    fs::create_dir(project.path().join("src")).unwrap();
    fs::write(project.path().join("src/main.vr"), source).unwrap();
    project
}

#[test]
fn simultaneous_same_coordinate_archives_remain_independent() {
    let name = format!("upload-{}", uuid::Uuid::new_v4().simple());
    let first_project = project(&name, b"first source");
    let second_project = project(&name, b"second source");
    let first = create_cog_tarball(first_project.path()).unwrap();
    let first_path = first.to_path_buf();
    let first_bytes = fs::read(&first).unwrap();
    let second = create_cog_tarball(second_project.path()).unwrap();
    let second_path = second.to_path_buf();
    assert_ne!(first_path, second_path);
    assert_eq!(fs::read(&first).unwrap(), first_bytes);
    assert_ne!(fs::read(&second).unwrap(), first_bytes);
    drop(first);
    assert!(!first_path.exists());
    assert!(
        second_path.exists(),
        "one upload must not remove another's bytes"
    );
    drop(second);
    assert!(!second_path.exists());
}

#[test]
fn archive_is_removed_when_later_publication_step_fails() {
    let project = project("archive-cleanup", b"source");
    let mut observed = None;
    let result = (|| -> crate::error::Result<()> {
        let archive = create_cog_tarball(project.path())?;
        observed = Some(archive.to_path_buf());
        assert!(archive.exists());
        Err(crate::error::CliError::Custom("rejected upload".into()))
    })();
    assert!(result.is_err());
    assert!(!observed.unwrap().exists());
}

#[test]
fn vanished_source_tree_is_a_packaging_error() {
    let project = TempDir::new().unwrap();
    let mut archive = tar::Builder::new(List::new());
    let result = add_directory_to_archive(&mut archive, &project.path().join("gone"), "src");
    assert!(
        result.is_err(),
        "a traversal error must not become an empty source archive"
    );
}

#[test]
fn source_root_must_be_a_directory() {
    let project = project("invalid-source-root", b"source");
    fs::remove_dir_all(project.path().join("src")).unwrap();
    fs::write(project.path().join("src"), b"not a directory").unwrap();
    assert!(
        create_cog_tarball(project.path())
            .unwrap_err()
            .to_string()
            .contains("src must be a directory")
    );
}

#[cfg(unix)]
fn symlink(target: impl AsRef<Path>, link: impl AsRef<Path>) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(unix)]
#[test]
fn source_symlinks_are_refused_including_dangling_links_and_directories() {
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret"), b"not package source").unwrap();
    for target in [
        outside.path().join("secret"),
        outside.path().join("missing"),
        outside.path().to_path_buf(),
    ] {
        let project = project("source-link", b"source");
        symlink(target, project.path().join("src/link"));
        let error = create_cog_tarball(project.path()).unwrap_err().to_string();
        assert!(error.contains("symlink"), "{error}");
    }
}

#[cfg(unix)]
#[test]
fn linked_source_root_is_refused() {
    let project = project("source-root-link", b"source");
    let outside = TempDir::new().unwrap();
    fs::remove_dir_all(project.path().join("src")).unwrap();
    symlink(outside.path(), project.path().join("src"));
    assert!(
        create_cog_tarball(project.path())
            .unwrap_err()
            .to_string()
            .contains("symlink")
    );
}

#[cfg(unix)]
#[test]
fn linked_manifest_and_optional_files_are_refused() {
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("document"), b"external document").unwrap();
    for name in [Manifest::MANIFEST_FILENAME, "README.md", "LICENSE"] {
        let project = project("document-link", b"source");
        let path = project.path().join(name);
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        symlink(outside.path().join("document"), path);
        let error = create_cog_tarball(project.path()).unwrap_err().to_string();
        assert!(error.contains("symlink"), "{error}");
    }
}

// macOS filesystems reject this filename before packaging can inspect it.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn non_utf8_source_names_are_not_silently_renamed() {
    use std::os::unix::ffi::OsStringExt;
    let project = project("invalid-filename", b"source");
    let name = std::ffi::OsString::from_vec(b"invalid-\xff.vr".to_vec());
    fs::write(project.path().join("src").join(name), b"source").unwrap();
    let error = create_cog_tarball(project.path()).unwrap_err().to_string();
    assert!(error.contains("not UTF-8"), "{error}");
}

#[cfg(unix)]
#[test]
fn archive_file_is_private_to_its_owner() {
    use std::os::unix::fs::PermissionsExt;
    let project = project("private-archive", b"source");
    let archive = create_cog_tarball(project.path()).unwrap();
    assert_eq!(
        fs::metadata(&archive).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
