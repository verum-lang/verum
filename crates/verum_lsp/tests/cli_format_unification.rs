//! Contract: the LSP `textDocument/formatting` route and `verum fmt`
//! produce byte-identical output for any input.
//!
//! Both surfaces eventually pipe through `verum fmt --stdin`, so
//! this test is the regression gate that catches any future
//! divergence — e.g. someone adding LSP-only post-processing.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use verum_lsp::cli_format::{format_via_cli, FmtSettings};

fn binary_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target");
    p.push("release");
    p.push("verum");
    p
}

fn run_cli_fmt_stdin(input: &str) -> String {
    let mut child = Command::new(binary_path())
        .args(["fmt", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("verum fmt --stdin spawn");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait_with_output");
    String::from_utf8(out.stdout).expect("UTF-8 stdout")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_and_lsp_routes_produce_identical_output() {
    let fixtures: &[(&str, &str)] = &[
        ("plain fn", "fn main() {}\n"),
        (
            "attributed fn",
            "@verify(formal)\npublic fn divide(a: Int, b: Int{ it != 0 }) -> Int { a / b }\n",
        ),
        (
            "type and impl",
            "type Point is { x: Int, y: Int };\n\nimplement Point {\n    fn x(self) -> Int { self.x }\n}\n",
        ),
        (
            "blank-line normalisation",
            "fn a() {}\n\n\n\n\nfn b() {}\n",
        ),
    ];

    let settings = FmtSettings {
        enabled: true,
        binary: Some(binary_path()),
    };

    for (label, src) in fixtures {
        let cli_output = run_cli_fmt_stdin(src);
        let lsp_output = format_via_cli(src, &settings, None)
            .await
            .unwrap_or_else(|| panic!("LSP format returned None for `{label}`"));
        assert_eq!(
            cli_output, lsp_output,
            "fixture `{label}`: CLI and LSP routes diverged.\n\
             === CLI ===\n{cli_output}\n=== LSP ===\n{lsp_output}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fmt_disabled_via_settings_returns_none() {
    let settings = FmtSettings {
        enabled: false,
        binary: Some(binary_path()),
    };
    let result = format_via_cli("fn main() {}\n", &settings, None).await;
    assert!(result.is_none(), "disabled settings should bypass the formatter");
}
