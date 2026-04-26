//! Cross-file rule tests.
//!
//! Each test stands up a small fixture project containing the
//! specific topology a cross-file rule fires on, runs the binary,
//! and asserts the expected diagnostic appears.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_lint_cross_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(
        dir.join("verum.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .expect("manifest");
    dir
}

fn run(dir: &PathBuf) -> String {
    let out = Command::new(binary())
        .args(["lint", "--no-cache", "--format", "json"])
        .current_dir(dir)
        .output()
        .expect("verum lint failed to spawn");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn count_rule(json_out: &str, rule: &str) -> usize {
    json_out
        .lines()
        .filter(|l| l.contains(&format!("\"rule\":\"{rule}\"")))
        .count()
}

// ============================================================
// circular-import
// ============================================================

#[test]
fn circular_import_fires_on_two_node_cycle() {
    let dir = make_dir("circular_2");
    std::fs::write(
        dir.join("src").join("a.vr"),
        "mount b;\n\npublic fn ping() { print(\"a\"); }\n",
    )
    .expect("a.vr");
    std::fs::write(
        dir.join("src").join("b.vr"),
        "mount a;\n\npublic fn pong() { print(\"b\"); }\n",
    )
    .expect("b.vr");

    let out = run(&dir);
    assert!(
        count_rule(&out, "circular-import") >= 1,
        "circular-import should fire on two-node cycle, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn circular_import_silent_on_dag() {
    let dir = make_dir("circular_dag");
    std::fs::write(
        dir.join("src").join("leaf.vr"),
        "public fn leaf() {}\n",
    )
    .expect("leaf.vr");
    std::fs::write(
        dir.join("src").join("middle.vr"),
        "mount leaf;\n\npublic fn middle() {}\n",
    )
    .expect("middle.vr");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "mount middle;\nmount leaf;\n\nfn main() {}\n",
    )
    .expect("main.vr");

    let out = run(&dir);
    assert_eq!(
        count_rule(&out, "circular-import"),
        0,
        "circular-import should NOT fire on a DAG, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================
// orphan-module
// ============================================================

#[test]
fn orphan_module_fires_on_isolated_file() {
    let dir = make_dir("orphan");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() {}\n",
    )
    .expect("main.vr");
    std::fs::write(
        dir.join("src").join("dangling.vr"),
        "public fn dangling() {}\n",
    )
    .expect("dangling.vr");

    let out = run(&dir);
    assert!(
        count_rule(&out, "orphan-module") >= 1,
        "orphan-module should fire on dangling.vr, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn orphan_module_silent_on_mounted_file() {
    let dir = make_dir("orphan_mounted");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "mount helper;\n\nfn main() {}\n",
    )
    .expect("main.vr");
    std::fs::write(
        dir.join("src").join("helper.vr"),
        "public fn helper() {}\n",
    )
    .expect("helper.vr");

    let out = run(&dir);
    assert_eq!(
        count_rule(&out, "orphan-module"),
        0,
        "orphan-module should NOT fire on a mounted file, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn orphan_module_skips_main_vr() {
    // main.vr is the entry point — it never needs to be mounted.
    let dir = make_dir("orphan_main_skipped");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() { print(\"hi\"); }\n",
    )
    .expect("main.vr");

    let out = run(&dir);
    assert_eq!(
        count_rule(&out, "orphan-module"),
        0,
        "orphan-module should skip main.vr, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================
// unused-public — opt-in via [lint.rules.unused-public].enabled
// ============================================================

#[test]
fn unused_public_silent_when_disabled() {
    let dir = make_dir("unused_public_disabled");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() {}\n",
    )
    .expect("main.vr");
    std::fs::write(
        dir.join("src").join("unused.vr"),
        "public fn never_called() -> Int { 0 }\n",
    )
    .expect("unused.vr");

    let out = run(&dir);
    assert_eq!(
        count_rule(&out, "unused-public"),
        0,
        "unused-public should be silent when not opted in, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unused_public_fires_when_enabled_in_config() {
    let dir = make_dir("unused_public_enabled");
    std::fs::write(
        dir.join("verum.toml"),
        "[package]\nname = \"unused_public_enabled\"\nversion = \"0.1.0\"\n\n[lint.rules.unused-public]\nenabled = true\n",
    )
    .expect("manifest");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() {}\n",
    )
    .expect("main.vr");
    std::fs::write(
        dir.join("src").join("unused.vr"),
        "public fn never_called_anywhere() -> Int { 0 }\n",
    )
    .expect("unused.vr");

    let out = run(&dir);
    assert!(
        count_rule(&out, "unused-public") >= 1,
        "unused-public should fire when enabled, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================
// unused-private
// ============================================================

#[test]
fn unused_private_fires_on_uncalled_local() {
    let dir = make_dir("priv_uncalled");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn helper() -> Int { 0 }\n\nfn main() {}\n",
    )
    .expect("main.vr");
    let out = run(&dir);
    assert!(
        count_rule(&out, "unused-private") >= 1,
        "unused-private should fire on `helper`, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unused_private_silent_when_used_in_same_file() {
    let dir = make_dir("priv_used");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn helper() -> Int { 0 }\n\nfn main() { let _ = helper(); }\n",
    )
    .expect("main.vr");
    let out = run(&dir);
    assert_eq!(
        count_rule(&out, "unused-private"),
        0,
        "unused-private should NOT fire when helper is called, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unused_private_skips_main() {
    let dir = make_dir("priv_main");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() { print(\"hi\"); }\n",
    )
    .expect("main.vr");
    let out = run(&dir);
    assert_eq!(
        count_rule(&out, "unused-private"),
        0,
        "unused-private must skip the `main` entry-point convention, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================
// dead-module
// ============================================================

#[test]
fn dead_module_fires_when_chain_does_not_reach_entry_point() {
    let dir = make_dir("dead_chain");
    // main.vr does NOT mount a; a mounts b. Both a and b are
    // unreachable from main.vr. orphan-module catches a (no one
    // mounts it); dead-module catches BOTH a and b (neither is
    // reachable from the entry point along the mount graph).
    std::fs::write(
        dir.join("src").join("main.vr"),
        "fn main() {}\n",
    )
    .expect("main.vr");
    std::fs::write(
        dir.join("src").join("a.vr"),
        "mount b;\n\npublic fn a_fn() {}\n",
    )
    .expect("a.vr");
    std::fs::write(
        dir.join("src").join("b.vr"),
        "public fn b_fn() {}\n",
    )
    .expect("b.vr");
    let out = run(&dir);
    assert!(
        count_rule(&out, "dead-module") >= 1,
        "dead-module should fire on the unreachable chain, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dead_module_silent_when_chain_reaches_entry_point() {
    let dir = make_dir("dead_reachable");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "mount a;\n\nfn main() {}\n",
    )
    .expect("main.vr");
    std::fs::write(
        dir.join("src").join("a.vr"),
        "mount b;\n\npublic fn a_fn() {}\n",
    )
    .expect("a.vr");
    std::fs::write(
        dir.join("src").join("b.vr"),
        "public fn b_fn() {}\n",
    )
    .expect("b.vr");
    let out = run(&dir);
    assert_eq!(
        count_rule(&out, "dead-module"),
        0,
        "dead-module should NOT fire when the chain is reachable, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================
// inconsistent-public-doc — opt-in
// ============================================================

#[test]
fn inconsistent_public_doc_silent_when_disabled() {
    let dir = make_dir("doc_disabled");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "/// documented\npublic fn a() {}\n\npublic fn b() {}\n\nfn main() {}\n",
    )
    .expect("main.vr");
    let out = run(&dir);
    assert_eq!(
        count_rule(&out, "inconsistent-public-doc"),
        0,
        "inconsistent-public-doc is opt-in, must be silent by default. got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inconsistent_public_doc_fires_when_enabled() {
    let dir = make_dir("doc_enabled");
    std::fs::write(
        dir.join("verum.toml"),
        "[package]\nname = \"doc_enabled\"\nversion = \"0.1.0\"\n\n[lint.rules.inconsistent-public-doc]\nenabled = true\n",
    )
    .expect("manifest");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "/// documented\npublic fn a() {}\n\npublic fn b() {}\n\nfn main() {}\n",
    )
    .expect("main.vr");
    let out = run(&dir);
    assert!(
        count_rule(&out, "inconsistent-public-doc") >= 1,
        "inconsistent-public-doc should fire when enabled and 1/2 docs missing, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================
// pub-exports-unsafe
// ============================================================

#[test]
fn pub_exports_unsafe_fires_on_unsafe_in_signature() {
    let dir = make_dir("pub_unsafe");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "public fn raw_buffer(data: &unsafe [Byte]) -> Int { 0 }\n\nfn main() {}\n",
    )
    .expect("main.vr");
    let out = run(&dir);
    assert!(
        count_rule(&out, "pub-exports-unsafe") >= 1,
        "pub-exports-unsafe should fire on `&unsafe` in signature, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pub_exports_unsafe_silent_on_safe_signature() {
    let dir = make_dir("pub_safe");
    std::fs::write(
        dir.join("src").join("main.vr"),
        "public fn safe_op(x: Int) -> Int { x + 1 }\n\nfn main() {}\n",
    )
    .expect("main.vr");
    let out = run(&dir);
    assert_eq!(
        count_rule(&out, "pub-exports-unsafe"),
        0,
        "pub-exports-unsafe should NOT fire on safe signature, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================
// mount-cycle-via-stdlib
// ============================================================

#[test]
fn mount_cycle_via_stdlib_fires_on_user_namespace_through_stdlib() {
    // User corpus has `app` namespace. A file under app/ mounts
    // `stdlib.app.something` — that re-entry into the user's own
    // top-level namespace via stdlib re-export is the smell.
    let dir = make_dir("mc_stdlib");
    std::fs::create_dir_all(dir.join("src").join("app")).expect("app dir");
    std::fs::write(
        dir.join("src").join("app").join("main.vr"),
        "mount stdlib.app.helpers;\n\nfn main() {}\n",
    )
    .expect("app/main.vr");
    let out = run(&dir);
    assert!(
        count_rule(&out, "mount-cycle-via-stdlib") >= 1,
        "mount-cycle-via-stdlib should fire on `stdlib.app.*` from a corpus with `app` namespace, got:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
