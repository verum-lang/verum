//! Included by the production cache module to exercise its actual key producer.
use super::{AotObjectCache, object_key};
use std::{
    fs,
    path::Path,
    time::{Duration, UNIX_EPOCH},
};

fn stamp(path: &Path) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1_000_000)))
        .unwrap();
}

#[test]
fn native_object_keys_distinguish_same_stamped_compilers_by_bytes() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("compiler-a");
    let changed = root.path().join("compiler-b");
    let copied = root.path().join("compiler-copy");
    fs::write(&first, b"compiler-A").unwrap();
    fs::write(&changed, b"compiler-B").unwrap();
    fs::copy(&first, &copied).unwrap();
    stamp(&first);
    stamp(&changed);
    assert_eq!(
        fs::metadata(&first).unwrap().len(),
        fs::metadata(&changed).unwrap().len()
    );
    assert_eq!(
        fs::metadata(&first).unwrap().modified().unwrap(),
        fs::metadata(&changed).unwrap().modified().unwrap()
    );
    let source = b"fn answer() -> Int { 73 }";
    let source_hash = blake3::hash(source);
    let stdlib = blake3::hash(b"stdlib source snapshot");
    let key = |path: &Path| {
        object_key(
            &crate::cache_identity::file(path).unwrap(),
            "target=A",
            source.len(),
            &source_hash,
            &stdlib,
        )
    };
    assert_ne!(key(&first), key(&changed));
    assert_eq!(key(&first), key(&copied));
}

#[test]
fn native_keys_preserve_source_target_and_stdlib_dependencies() {
    let compiler = blake3::hash(b"compiler");
    let source = blake3::hash(b"source");
    let stdlib = blake3::hash(b"stdlib");
    let key = object_key(&compiler, "target=A", 6, &source, &stdlib);
    for other in [
        object_key(&compiler, "target=B", 6, &source, &stdlib),
        object_key(&compiler, "target=A", 6, &blake3::hash(b"change"), &stdlib),
        object_key(
            &compiler,
            "target=A",
            6,
            &source,
            &blake3::hash(b"changed stdlib"),
        ),
    ] {
        assert_ne!(key, other);
    }
}

#[test]
#[cfg(unix)]
fn missing_executable_cannot_prepare_native_cache() {
    const CHILD: &str = "VERUM_AOT_CACHE_IDENTITY_TEST_CHILD";
    if let Some(copy) = std::env::var_os(CHILD) {
        let actual = std::env::current_exe().unwrap();
        assert_eq!(actual, std::path::PathBuf::from(copy));
        fs::remove_file(&actual).unwrap();
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("main.vr");
        fs::write(&source, "fn main() -> Int { 73 }").unwrap();
        for _ in 0..2 {
            assert!(AotObjectCache::prepare(root.path(), &source, "same-config").is_none());
        }
        assert!(!root.path().join("aot-object-cache").exists());
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let copy = root.path().canonicalize().unwrap().join("cache-test");
    fs::copy(std::env::current_exe().unwrap(), &copy).unwrap();
    let output = std::process::Command::new(&copy)
        .args(["--exact", "pipeline::aot_object_cache::identity_tests::missing_executable_cannot_prepare_native_cache", "--nocapture"])
        .env(CHILD, &copy).output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        std::str::from_utf8(&output.stdout).unwrap_or("<invalid UTF-8>"),
        std::str::from_utf8(&output.stderr).unwrap_or("<invalid UTF-8>")
    );
    assert!(!copy.exists(), "the child must remove its own executable");
}
