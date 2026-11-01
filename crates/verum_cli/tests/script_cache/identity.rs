use super::{compiler_file_identity, key_with_identity};
use std::{
    fs,
    time::{Duration, UNIX_EPOCH},
};

fn stamp(path: &std::path::Path, seconds: u64) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(seconds)))
        .unwrap();
}

#[test]
fn changed_bytes_cannot_reuse_identity_even_with_identical_length_and_timestamp() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("compiler-a");
    let b = dir.path().join("compiler-b");
    fs::write(&a, b"compiler-A").unwrap();
    fs::write(&b, b"compiler-B").unwrap();
    stamp(&a, 1_000_000);
    stamp(&b, 1_000_000);
    assert_eq!(
        fs::metadata(&a).unwrap().len(),
        fs::metadata(&b).unwrap().len()
    );
    assert_eq!(
        fs::metadata(&a).unwrap().modified().unwrap(),
        fs::metadata(&b).unwrap().modified().unwrap()
    );
    let a = compiler_file_identity(&a).unwrap();
    let b = compiler_file_identity(&b).unwrap();
    assert_ne!(a, b);
    assert_ne!(
        key_with_identity(b"print(42);", "0.1", &[], a.as_bytes()),
        key_with_identity(b"print(42);", "0.1", &[], b.as_bytes())
    );
}

#[test]
fn identical_compiler_bytes_survive_copy_and_timestamp_changes() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("original");
    let b = dir.path().join("copy");
    fs::write(&a, b"compiler-A").unwrap();
    fs::copy(&a, &b).unwrap();
    stamp(&a, 1_000_000);
    stamp(&b, 2_000_000);
    assert_ne!(
        fs::metadata(&a).unwrap().modified().unwrap(),
        fs::metadata(&b).unwrap().modified().unwrap()
    );
    let a = compiler_file_identity(&a).unwrap();
    let b = compiler_file_identity(&b).unwrap();
    assert_eq!(a, blake3::hash(b"compiler-A"));
    assert_eq!(a, b);
    assert_eq!(
        key_with_identity(b"print(42);", "0.1", &[], a.as_bytes()),
        key_with_identity(b"print(42);", "0.1", &[], b.as_bytes())
    );
}

#[test]
fn invalid_compiler_files_have_no_content_identity() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty");
    fs::write(&empty, b"").unwrap();
    for path in [dir.path().join("missing"), empty, dir.path().to_owned()] {
        let identity = compiler_file_identity(&path);
        assert!(identity.is_err(), "{}", path.display());
    }
}

#[test]
#[cfg(unix)]
fn missing_running_executable_cannot_create_a_version_only_key() {
    const CHILD: &str = "VERUM_CACHE_IDENTITY_TEST_CHILD";
    if let Some(owned_copy) = std::env::var_os(CHILD) {
        let actual = std::env::current_exe().unwrap();
        assert_eq!(actual, std::path::PathBuf::from(owned_copy));
        fs::remove_file(&actual).unwrap();
        assert!(super::key_for(b"print(42);", "same-version", &[]).is_err());
        // Failure remains a failure in the process-wide identity cache.
        assert!(super::key_for(b"print(42);", "same-version", &[]).is_err());
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let owned_copy = dir.path().canonicalize().unwrap().join("compiler-test");
    fs::copy(std::env::current_exe().unwrap(), &owned_copy).unwrap();
    let output = std::process::Command::new(&owned_copy)
        .args(["--exact", "script::cache::identity_tests::missing_running_executable_cannot_create_a_version_only_key", "--nocapture"])
        .env(CHILD, &owned_copy).output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        std::str::from_utf8(&output.stdout).unwrap_or("<invalid UTF-8>"),
        std::str::from_utf8(&output.stderr).unwrap_or("<invalid UTF-8>")
    );
    assert!(
        !owned_copy.exists(),
        "child must actually remove its own test copy"
    );
}
