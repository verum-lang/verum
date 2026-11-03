//! Entry mode remains explicit even when the same source has cached script VBC.
use std::{io::Write, process::{Command, Stdio}};

#[test]
fn named_stdin_preserves_declared_mode_across_cached_runs() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("entry.vr");
    let source = format!("print(42); // {}\n", uuid::Uuid::new_v4());
    for (mode, accepted) in [("source", false), ("script", true), ("source", false)] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_verum"))
            .args(["run", "--interp", "--stdin-filepath"])
            .arg(&path).args(["--stdin-mode", mode, "-"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().unwrap();
        child.stdin.take().unwrap().write_all(source.as_bytes()).unwrap();
        let result = child.wait_with_output().unwrap();
        let diagnostic = String::from_utf8_lossy(&result.stderr);
        assert_eq!(result.status.success(), accepted, "{mode}: {diagnostic}");
        if accepted {
            assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "42");
        } else {
            assert!(diagnostic.contains("expected item"), "must be a real syntax refusal: {diagnostic}");
        }
        assert!(!path.exists(), "stdin overlay must not write the project source");
    }
}
