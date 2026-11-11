//! An invalid call must not disable ownership diagnostics in subsequent source.
use std::{fs, process::Command};

fn check(body: &str, accepted: bool, expected: &[&str]) {
    let root = tempfile::tempdir().unwrap();
    let core = root.path().join("core");
    fs::create_dir(&core).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module prelude {}").unwrap();
    let entry = root.path().join("main.vr");
    fs::write(
        &entry,
        format!(
            "@no_implicit_prelude\nmodule app.main;\n\
         type affine Parcel is {{value: Int}};\n\
         fn need(x: Int) {{}}\n{body}\nfn main() {{}}"
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_verum"))
        .arg("check")
        .arg(&entry)
        .current_dir(root.path())
        .env("VERUM_STDLIB_PATH", core)
        .env("VERUM_ALLOW_STDLIB_SOURCE_FALLBACK", "1")
        .env("VERUM_NO_OBJECT_CACHE", "1")
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    let messages = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.success(), accepted, "{messages}");
    for message in expected {
        assert!(messages.contains(message), "missing {message}: {messages}");
    }
}

#[test]
fn invalid_calls_preserve_following_ownership_diagnostics() {
    for body in [
        "fn bad(original: Parcel) {need(true); (original, original);}",
        "fn bad() {need(true);} fn later(original: Parcel) {(original, original);}",
        "fn bad(original: Parcel) {(|x: Int| x)(true); (original, original);}",
    ] {
        check(body, false, &["Bool", "Int", "E310", "used after move"]);
    }
    check(
        "fn valid(original: Parcel) {need(1); let moved = original; moved.value;}",
        true,
        &[],
    );
}
