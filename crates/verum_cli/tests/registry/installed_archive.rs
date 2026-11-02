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
