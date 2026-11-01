//! Keep the graphical workspace and terminal REPL independently usable.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn supported_interactive_commands_remain_discoverable() {
    for command in ["studio", "repl", "proof-repl"] {
        let output = Command::new(env!("CARGO_BIN_EXE_verum"))
            .args([command, "--help"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{command}: {output:?}");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_verum"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("verum studio ."), "{help}");
    assert!(!help.to_lowercase().contains("playbook"), "{help}");
}

#[test]
fn removed_notebook_commands_are_rejected() {
    for command in ["playbook", "play", "playbook-convert"] {
        let output = Command::new(env!("CARGO_BIN_EXE_verum"))
            .args([command, "--help"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{command}: {output:?}");
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("unrecognized subcommand"), "{error}");
    }
}

#[test]
fn terminal_repl_retains_declarations_and_evaluates_them() {
    let workspace = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_verum"))
        .arg("repl")
        .current_dir(workspace.path())
        .env_remove("VERUM_STDLIB_PATH")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"fn twice(x: Int) -> Int { x * 2 }\nprint(twice(21));\n:quit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("42"), "{stdout}");
    assert!(stdout.contains("Goodbye!"), "{stdout}");
    assert!(!stdout.contains("error:"), "{stdout}");
}
