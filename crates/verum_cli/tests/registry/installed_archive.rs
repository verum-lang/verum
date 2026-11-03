use super::{CacheManager, member_key};
use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

#[test]
fn portable_members_and_legacy_root() {
    assert_eq!(member_key(Path::new("./"), true).as_deref(), Some(""));
    assert_eq!(
        member_key(Path::new("./src/Api.vr"), false).as_deref(),
        Some("src/api.vr")
    );
    for name in [
        "",
        ".",
        "..",
        "src/../outside",
        "/outside",
        "C:/file",
        r"src\file",
        "src/Bad.",
        "src/con.txt",
        "src/LPt9",
        "src/a:b",
        "a\nb",
    ] {
        assert!(member_key(Path::new(name), false).is_none(), "{name}");
    }
}

#[test]
fn legacy_cache_archive_is_verified_before_extraction() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = CacheManager::new(tmp.path().join("cache")).unwrap();
    let source = tmp.path().join("source");
    fs::create_dir_all(source.join("src")).unwrap();
    fs::write(source.join("src/api.vr"), b"public fn api() {}").unwrap();
    let archive = cache.create_archive(&source, "widget", "1.2.3").unwrap();
    let digest = hex::encode(Sha256::digest(fs::read(&archive).unwrap()));
    let out = tmp.path().join("out");
    cache
        .extract_locked_source("widget", "1.2.3", &digest, &out)
        .unwrap();
    assert_eq!(
        fs::read(out.join("src/api.vr")).unwrap(),
        b"public fn api() {}"
    );
    let other = tmp.path().join("other");
    fs::write(&archive, b"different").unwrap();
    assert!(
        cache
            .extract_locked_source("widget", "1.2.3", &digest, &other)
            .is_err()
    );
    fs::remove_file(&archive).unwrap();
    assert!(
        cache
            .extract_locked_source("widget", "1.2.3", &digest, &other)
            .is_err()
    );
    assert!(!other.exists());
}

#[test]
fn pinned_symlinks_and_duplicate_members_are_refused_before_any_writes() {
    for link in [true, false] {
        let tmp = tempfile::tempdir().unwrap();
        let cache = CacheManager::new(tmp.path().join("cache")).unwrap();
        let archive = cache.get_cog_path("widget", "1.2.3");
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        let mut tar = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_mode(0o644);
        tar.append_data(&mut header, "src/API.vr", &b"x"[..])
            .unwrap();
        if link {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            tar.append_link(&mut header, "outside", "../../target")
                .unwrap();
        } else {
            tar.append_data(&mut header, "src/api.vr", &b"z"[..])
                .unwrap();
        }
        let bytes = tar.into_inner().unwrap().finish().unwrap();
        fs::write(&archive, &bytes).unwrap();
        let digest = hex::encode(Sha256::digest(bytes));
        let out = tmp.path().join("out");
        assert!(
            cache
                .extract_locked_source("widget", "1.2.3", &digest, &out)
                .is_err()
        );
        assert!(!out.exists());
    }
}

fn cached_members(cache: &CacheManager, members: &[(&str, &[u8])]) -> (std::path::PathBuf, String) {
    let archive = cache.get_cog_path("widget", "1.2.3");
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    let mut tar = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for (name, contents) in members {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        tar.append_data(&mut header, name, *contents).unwrap();
    }
    let bytes = tar.into_inner().unwrap().finish().unwrap();
    let digest = hex::encode(Sha256::digest(&bytes));
    fs::write(&archive, bytes).unwrap();
    (archive, digest)
}

#[test]
fn source_extraction_never_replaces_an_existing_file() {
    for locked in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let cache = CacheManager::new(tmp.path().join("cache")).unwrap();
        let (archive, digest) = cached_members(&cache, &[("src/api.vr", b"archive")]);
        let out = tmp.path().join("out");
        fs::create_dir_all(out.join("src")).unwrap();
        fs::write(out.join("src/api.vr"), b"existing").unwrap();
        let result = if locked {
            cache.extract_locked_source("widget", "1.2.3", &digest, &out)
        } else {
            cache.extract(&archive, &out)
        };
        assert!(result.is_err(), "locked={locked}");
        assert_eq!(fs::read(out.join("src/api.vr")).unwrap(), b"existing");
    }
}

#[test]
fn unpinned_extraction_refuses_duplicate_members_instead_of_last_write_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = CacheManager::new(tmp.path().join("cache")).unwrap();
    let (archive, _) = cached_members(
        &cache,
        &[("src/api.vr", b"first"), ("src/api.vr", b"second")],
    );
    let out = tmp.path().join("out");
    assert!(cache.extract(&archive, &out).is_err());
    assert_eq!(fs::read(out.join("src/api.vr")).unwrap(), b"first");
}

#[cfg(unix)]
#[test]
fn source_extraction_never_unlinks_an_existing_symlink() {
    for locked in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let cache = CacheManager::new(tmp.path().join("cache")).unwrap();
        let (archive, digest) = cached_members(&cache, &[("src/api.vr", b"archive")]);
        let out = tmp.path().join("out");
        let target = tmp.path().join("target");
        fs::create_dir_all(out.join("src")).unwrap();
        fs::write(&target, b"existing").unwrap();
        std::os::unix::fs::symlink(&target, out.join("src/api.vr")).unwrap();
        let result = if locked {
            cache.extract_locked_source("widget", "1.2.3", &digest, &out)
        } else {
            cache.extract(&archive, &out)
        };
        assert!(result.is_err(), "locked={locked}");
        assert_eq!(fs::read_link(out.join("src/api.vr")).unwrap(), target);
        assert_eq!(fs::read(&target).unwrap(), b"existing");
    }
}

#[test]
fn locked_extraction_obeys_actual_filesystem_unicode_identity() {
    for (first, second) in [
        ("café.vr", "cafe\u{301}.vr"),
        ("É.vr", "é.vr"),
        ("Ω.vr", "ω.vr"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let cache = CacheManager::new(tmp.path().join("cache")).unwrap();
        // Establish the filesystem's own identity rule without assuming an OS,
        // normalization form or Unicode version. These names pass lexical guards.
        assert_ne!(
            member_key(Path::new(first), false),
            member_key(Path::new(second), false)
        );
        let probe = tmp.path().join("probe");
        fs::create_dir(&probe).unwrap();
        fs::write(probe.join(first), b"probe").unwrap();
        let collision = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(probe.join(second))
        {
            Ok(_) => false,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => true,
            Err(error) => panic!("filesystem probe failed: {error}"),
        };
        let (_, digest) = cached_members(&cache, &[(first, b"first"), (second, b"second")]);
        let out = tmp.path().join("out");
        let result = cache.extract_locked_source("widget", "1.2.3", &digest, &out);
        assert_eq!(
            result.is_err(),
            collision,
            "{first:?}/{second:?}: {result:?}"
        );
        assert_eq!(fs::read(out.join(first)).unwrap(), b"first");
        if !collision {
            assert_eq!(fs::read(out.join(second)).unwrap(), b"second");
        }
    }
}

#[test]
fn locked_extraction_preserves_distinct_unicode_members() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = CacheManager::new(tmp.path().join("cache")).unwrap();
    let (_, digest) = cached_members(
        &cache,
        &[("src/λ.vr", b"lambda"), ("src/данные.vr", b"data")],
    );
    let out = tmp.path().join("out");
    cache
        .extract_locked_source("widget", "1.2.3", &digest, &out)
        .unwrap();
    assert_eq!(fs::read(out.join("src/λ.vr")).unwrap(), b"lambda");
    assert_eq!(fs::read(out.join("src/данные.vr")).unwrap(), b"data");
}
