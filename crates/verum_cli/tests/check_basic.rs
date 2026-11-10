//! `verum check` end-to-end exit-code + diagnostic contract.
//!
//! `verum check` runs lex → parse → type-check on the project,
//! skipping VBC/LLVM codegen. The CLI contract this test pins down:
//!
//!  * Clean project → exit 0, no diagnostic-formatted output on stderr.
//!  * Parse error → exit non-zero, stderr identifies the offending file.
//!  * Unknown identifier (post-parse type-check) → exit non-zero,
//!  stderr surfaces the failing name.
//!
//! Pre-commit hooks, CI gates, and IDE "save and check" workflows rely
//! on the exit code being a binary signal — locking it down is the
//! same hardening contract `fmt --check` follows.
//!
//! Test isolation: each fixture is built in `$TMPDIR/verum_check_*`
//! with a dedicated PID-suffixed name to avoid cross-test contamination
//! when run in parallel.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str, body: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_check_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(
        dir.join("verum.toml"),
        format!("[cog]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .expect("manifest");
    std::fs::write(dir.join("src").join("main.vr"), body).expect("main.vr");
    dir
}

#[test]
fn check_exits_zero_on_valid_program() {
    // Smallest valid Verum program: an empty main(). Lex + parse +
    // type-check all succeed, no diagnostics.
    let dir = make_fixture("valid", "fn main() {}\n");
    let out = Command::new(binary())
        .args(["check"])
        .current_dir(&dir)
        .output()
        .expect("verum check failed to spawn");
    assert!(
        out.status.success(),
        "verum check should exit 0 on a valid program. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_exits_nonzero_on_parse_error() {
    // Unbalanced braces — the lexer accepts the bytes, the parser
    // fails at end-of-input expecting `}`.
    let dir = make_fixture("parse_err", "fn main() { let x = ;\n");
    let out = Command::new(binary())
        .args(["check"])
        .current_dir(&dir)
        .output()
        .expect("verum check failed to spawn");
    assert!(
        !out.status.success(),
        "verum check should exit non-zero on a parse error. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Diagnostic should mention the offending file — accept either the
    // full basename `main.vr` or the bare module name `main` (the
    // location formatter renders `at <module>:<line>:<col>` which
    // identifies the file via its module name).
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stderr}{stdout}");
    assert!(
        combined.contains("main.vr") || combined.contains("main:"),
        "parse error diagnostic should reference main.vr or main:; got: {combined}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_exits_nonzero_on_unknown_identifier() {
    // Reference an undeclared name. Parse succeeds; type-check fails.
    // This pins down that the type-check phase actually runs (not just
    // a lex-only fast path).
    let dir = make_fixture(
        "unknown_id",
        "fn main() { let x = some_undeclared_name; }\n",
    );
    let out = Command::new(binary())
        .args(["check"])
        .current_dir(&dir)
        .output()
        .expect("verum check failed to spawn");
    assert!(
        !out.status.success(),
        "verum check should exit non-zero on an unknown identifier. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_does_not_emit_codegen_artifacts() {
    // `verum check` must not emit a binary or VBC bundle. Pre-existing
    // contract: the fast-path is type-only, so the build directory
    // should be absent (or at least without compiled artefacts).
    let dir = make_fixture("no_artifacts", "fn main() {}\n");
    let out = Command::new(binary())
        .args(["check"])
        .current_dir(&dir)
        .output()
        .expect("verum check failed to spawn");
    assert!(out.status.success());

    // No binary in any of the conventional output locations.
    for candidate in &["target/debug/no_artifacts", "build/no_artifacts"] {
        let p = dir.join(candidate);
        assert!(
            !p.exists(),
            "verum check should NOT produce a compiled binary at {p:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// Source comments describe VCS expectations only to the VCS runner. Ordinary
// checking reports the actual selected phase, including in editor/agent jobs.
#[test]
fn source_directives_cannot_skip_or_invert_parse_failures() {
    for (index, header) in [
        "// @skip: fixture",
        "// @expect: errors",
        "// @test: parse-fail",
        "// @test: typecheck-fail",
    ]
    .iter()
    .enumerate()
    {
        let dir = make_fixture(
            &format!("directive_parse_{index}"),
            &format!("{header}\nfn main() {{ let x = ;\n"),
        );
        let out = Command::new(binary())
            .args(["check", "src/main.vr", "--parse-only"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "{header} suppressed a parse error: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn negative_test_comments_do_not_invert_a_successful_parse() {
    let dir = make_fixture(
        "directive_valid",
        "// @test: parse-fail\n// @expect: errors\nfn main() {}\n",
    );
    let out = Command::new(binary())
        .args(["check", "src/main.vr", "--parse-only"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "comments changed a successful parse: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_the_explicit_parse_option_can_omit_type_checking() {
    let dir = make_fixture(
        "directive_phase",
        "// @test: parse-pass\nfn main() { let x = undeclared_check_sentinel; }\n",
    );
    let checked = Command::new(binary())
        .args(["check", "src/main.vr"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        !checked.status.success(),
        "source comments weakened checking: {} {}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(
        String::from_utf8_lossy(&checked.stderr).contains("undeclared_check_sentinel"),
        "expected the actual type-check diagnostic: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let parsed = Command::new(binary())
        .args(["check", "src/main.vr", "--parse-only"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        parsed.status.success(),
        "explicit parse-only mode must retain its documented meaning"
    );
    let _ = std::fs::remove_dir_all(dir);
}

fn write_source_core(root: &Path, modules: &[(&str, &str)]) -> PathBuf {
    let core = root.join("core");
    std::fs::create_dir_all(core.join("base")).unwrap();
    let mut declarations =
        String::from("module core; public module base; public module prelude {}");
    for (name, source) in modules {
        declarations.push_str(&format!(" public module {name};"));
        std::fs::write(core.join(format!("{name}.vr")), source).unwrap();
    }
    std::fs::write(core.join("mod.vr"), declarations).unwrap();
    std::fs::write(
        core.join("base/mod.vr"),
        "module core.base; public module protocols;",
    )
    .unwrap();
    std::fs::write(
        core.join("base/protocols.vr"),
        "module core.base.protocols;",
    )
    .unwrap();
    core
}

fn check_with_source_core(core: &Path, file: &Path, source: &str) -> Output {
    std::fs::write(file, source).unwrap();
    Command::new(binary())
        .arg("check")
        .arg(file)
        .current_dir(file.parent().unwrap())
        .env("VERUM_STDLIB_PATH", core)
        .env("VERUM_ALLOW_STDLIB_SOURCE_FALLBACK", "1")
        .output()
        .unwrap()
}

#[test]
fn protocol_signatures_resolve_the_entry_files_source_mounts() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let core = write_source_core(
        root,
        &[(
            "source_provider",
            "module core.source_provider; public type EntryTag is Ready | Busy; type PrivateTag is Hidden;",
        )],
    );
    let project = root.join("app");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("verum.toml"),
        "[cog]\nname = \"entry_scope\"\nversion = \"0.1.0\"\nedition = \"2026\"\n",
    )
    .unwrap();

    let standalone = root.join("standalone.vr");
    let entry = project.join("src/main.vr");
    for name in ["EntryTag", "core.source_provider.EntryTag"] {
        let source = format!(
            "mount core.source_provider.{{EntryTag, Ready}};\n\
             type Provider is protocol {{ fn tag(&self) -> {name}; }};\n\
             type Peer is {{ count: Int }};\n\
             implement Provider for Peer {{ fn tag(&self) -> {name} {{ Ready }} }}\n\
             fn main() {{}}"
        );
        for path in [&standalone, &entry] {
            let output = check_with_source_core(&core, path, &source);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    let hidden = check_with_source_core(
        &core,
        &standalone,
        "mount core.source_provider.EntryTag; type Provider is protocol { fn tag(&self) -> core.source_provider.PrivateTag; }; fn main() {}",
    );
    let error = String::from_utf8_lossy(&hidden.stderr);
    assert!(!hidden.status.success());
    assert!(
        error.contains("PrivateTag") && error.contains("visibility error"),
        "{error}"
    );
}

#[test]
fn inherent_signatures_resolve_types_in_their_defining_source_module() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let events = r#"
        module core.events;
        public type Event<T> is { public content: T };
        public type Closed is { public bytes: Int };
    "#;
    let worker = r#"
        module core.worker;
        mount core.events.Event as ImportedEvent;
        public type Outcome<T> is { public sample: T };
        public type Failure is Closed;
        public type Worker<T> is { public value: T };
        implement<T> Worker<T> {
            public fn failure(&self) -> Failure { Closed }
            public fn local(&self, x: T) -> Outcome<T> { Outcome { sample: x } }
            public fn foreign(&self, x: T) -> ImportedEvent<T> { ImportedEvent { content: x } }
        }
    "#;
    let user = r#"
        mount core.events.Closed;
        mount core.worker.Worker;
        type Outcome<T> is { mistaken: T };
        type ImportedEvent<T> is { mistaken: T };
        type ConsumerOnly<T> is { content: T };
        fn main() {
            let worker = Worker { value: 17 };
            let failure = worker.failure();
            assert_eq(worker.local(31).sample, 31);
            assert_eq(worker.foreign(37).content, 37);
        }
    "#;
    let core = write_source_core(root, &[("events", events), ("worker", worker)]);
    let entry = root.join("entry.vr");
    let checked = check_with_source_core(&core, &entry, user);
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );

    let invalid_worker = worker.replace("-> ImportedEvent<T>", "-> ConsumerOnly<T>");
    std::fs::write(core.join("worker.vr"), invalid_worker).unwrap();
    let missing = check_with_source_core(&core, &entry, user);
    let error = String::from_utf8_lossy(&missing.stderr);
    assert!(!missing.status.success());
    assert!(
        error.contains("type not found: ConsumerOnly") && error.contains("core/worker.vr"),
        "{error}"
    );
}
