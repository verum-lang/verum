//! Automatic archive identity uses the actual build-script hash implementation.
//! Policy controls include a dependency-free Cargo fixture; none bake an archive.

#[allow(dead_code)]
#[path = "../build.rs"]
mod build_script;
#[path = "../build_support/stdlib_cache.rs"]
mod stdlib_cache;

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use verum_common::{List, Set, Text};

const SCHEMA: &str = "unchanged-wire-schema";
const CORE: &[(&str, &[u8])] = &[("mod.vr", b"public type Identity<T> is T;")];
const PARSER_DECLARATIONS: &str = "crates/verum_fast_parser/src/decl.rs";
const PARSER_MANIFEST: &str = "crates/verum_fast_parser/Cargo.toml";
const PARSER_SOURCE: &str = include_str!("../../verum_fast_parser/src/decl.rs");

fn write(root: &Path, relative: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn key(root: &Path, schema: &str, core: &[(&str, &[u8])]) -> (blake3::Hash, List<PathBuf>) {
    let mut dependencies = List::new();
    let key = stdlib_cache::compute_archive_key(root, schema, core.iter().copied(), |path| {
        dependencies.push(path.to_owned());
    });
    (key, dependencies)
}

#[test]
fn unchanged_inputs_keep_the_key_across_runs_and_checkout_roots() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    for root in [first.path(), second.path()] {
        write(root, PARSER_DECLARATIONS, PARSER_SOURCE);
    }
    let expected = key(first.path(), SCHEMA, CORE).0;
    assert_eq!(expected, key(first.path(), SCHEMA, CORE).0);
    assert_eq!(expected, key(second.path(), SCHEMA, CORE).0);
}

#[test]
fn declaration_parameter_classification_changes_the_key_without_core_or_schema_changes() {
    let root = tempfile::tempdir().unwrap();
    let fixed = "let body = if is_alias_syntax || is_parameter_alias {";
    let previous = "let body = if is_alias_syntax {";
    assert_eq!(PARSER_SOURCE.matches(fixed).count(), 1);
    // This is the actual T1720 branch condition. Disabling that condition
    // classifies `Identity<T> is T` as a marker rather than a generic alias.
    // It is a semantic producer edit, not a core/source or wire-schema edit.
    write(
        root.path(),
        PARSER_DECLARATIONS,
        PARSER_SOURCE.replace(fixed, previous),
    );
    let before = key(root.path(), SCHEMA, CORE).0;
    write(root.path(), PARSER_DECLARATIONS, PARSER_SOURCE);
    let after = key(root.path(), SCHEMA, CORE).0;
    assert_ne!(
        before, after,
        "parser-only alias semantics must invalidate the archive"
    );
}

#[test]
fn missing_parser_input_is_tracked_and_differs_from_an_empty_file() {
    let root = tempfile::tempdir().unwrap();
    let (missing, dependencies) = key(root.path(), SCHEMA, CORE);
    assert!(dependencies.contains(&root.path().join(PARSER_DECLARATIONS)));
    write(root.path(), PARSER_DECLARATIONS, b"");
    assert_ne!(missing, key(root.path(), SCHEMA, CORE).0);
    fs::remove_file(root.path().join(PARSER_DECLARATIONS)).unwrap();
    assert_eq!(missing, key(root.path(), SCHEMA, CORE).0);
}

#[test]
fn parser_manifest_changes_invalidate_the_archive() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        PARSER_MANIFEST,
        "[features]\nparser-policy = []\n",
    );
    let before = key(root.path(), SCHEMA, CORE).0;
    write(
        root.path(),
        PARSER_MANIFEST,
        "[features]\nparser-policy = [\"checked\"]\n",
    );
    assert_ne!(before, key(root.path(), SCHEMA, CORE).0);
}

#[test]
fn wire_schema_and_core_contents_remain_independent_inputs() {
    let root = tempfile::tempdir().unwrap();
    let before = key(root.path(), SCHEMA, CORE).0;
    assert_ne!(before, key(root.path(), "next-wire-schema", CORE).0);
    assert_ne!(
        before,
        key(
            root.path(),
            SCHEMA,
            &[("mod.vr", b"public type Flag is On;")]
        )
        .0
    );
    assert_ne!(
        before,
        key(root.path(), SCHEMA, &[("other.vr", CORE[0].1)]).0
    );
}

#[test]
fn producer_roster_covers_every_fast_parser_source_and_reports_each_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let (_, dependencies) = key(root, SCHEMA, CORE);
    let reported: Set<_> = dependencies.iter().collect();
    assert_eq!(
        reported.len(),
        dependencies.len(),
        "duplicate producer paths"
    );
    let mut expected = Set::new();
    expected.insert(root.join(PARSER_MANIFEST));
    let mut pending = List::from_iter([root.join("crates/verum_fast_parser/src")]);
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "rs")
            {
                expected.insert(entry.path());
            }
        }
    }
    let parser_paths: Set<_> = dependencies
        .into_iter()
        .filter(|path| path.starts_with(root.join("crates/verum_fast_parser")))
        .collect();
    assert_eq!(
        parser_paths, expected,
        "new parser files must join the producer roster"
    );
}

struct OwnedChild(Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn no_auto_still_reports_parser_dependencies_without_refreshing() {
    const CHILD: &str = "VERUM_CACHE_IDENTITY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        build_script::main();
        return;
    }
    // Exercise the actual build-script entry point in an isolated process.
    // An empty PATH also makes an accidental nested Cargo invocation fail
    // instead of allowing this control to bake or touch another target.
    for (flag, value) in [
        ("VERUM_NO_AUTO_PRECOMPILE", "1"),
        ("VERUM_NO_AUTO_PRECOMPILE", "0"),
        ("DOCS_RS", "1"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("out");
        let target = root.path().join("target");
        let manifest = root.path().join("crates/verum_compiler");
        fs::create_dir_all(&output).unwrap();
        fs::create_dir_all(&manifest).unwrap();
        write(root.path(), "core/mod.vr", CORE[0].1);
        write(root.path(), PARSER_DECLARATIONS, PARSER_SOURCE);
        let stdout = root.path().join("stdout.log");
        let stderr = root.path().join("stderr.log");
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "no_auto_still_reports_parser_dependencies_without_refreshing",
                "--nocapture",
            ])
            .env_remove("VERUM_NO_AUTO_PRECOMPILE")
            .env_remove("DOCS_RS")
            .env_remove("VERUM_ALLOW_STALE_STDLIB")
            .env(flag, value)
            .env(CHILD, "1")
            .env("PATH", root.path().join("no-executables"))
            .env("OUT_DIR", &output)
            .env("CARGO_MANIFEST_DIR", &manifest)
            .env("CARGO_TARGET_DIR", &target)
            .current_dir(root.path())
            .stdout(Stdio::from(fs::File::create(&stdout).unwrap()))
            .stderr(Stdio::from(fs::File::create(&stderr).unwrap()))
            .spawn()
            .unwrap();
        let mut child = OwnedChild(child);
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "build-script probe exceeded ten seconds"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let stdout: Text = fs::read_to_string(stdout).unwrap().into();
        let stderr: Text = fs::read_to_string(stderr).unwrap().into();
        assert!(
            status.success(),
            "{flag}={value}: {status}; {stdout}; {stderr}"
        );
        let dependency = format!(
            "cargo:rerun-if-changed={}",
            root.path().join(PARSER_DECLARATIONS).display()
        );
        assert!(
            stdout
                .lines()
                .iter()
                .any(|line| line.as_str() == dependency.as_str()),
            "{flag}={value}: {stdout}"
        );
        for policy in ["VERUM_NO_AUTO_PRECOMPILE", "DOCS_RS"] {
            let dependency = format!("cargo:rerun-if-env-changed={policy}");
            assert!(stdout.lines().iter().any(|line| line.as_str() == dependency),
                "{flag}={value}: {stdout}");
        }
        let checksum = format!(
            "cargo:rerun-if-changed={}",
            target
                .join("precompiled-stdlib/runtime.vbca.checksum")
                .display()
        );
        assert!(
            stdout
                .lines()
                .iter()
                .any(|line| line.as_str() == checksum.as_str())
        );
        assert!(!stdout.contains("Refreshing stdlib precompile"));
        assert!(!stdout.contains("Stdlib precompile cache HIT"));
        assert!(
            !target.exists(),
            "disabled precompile must not initialize a target or cache"
        );
        assert!(output.join("stdlib_runtime.vbca").is_file());
        assert_eq!(
            fs::metadata(output.join("stdlib_runtime.vbca"))
                .unwrap()
                .len(),
            0
        );
    }
}

/// Cargo must observe flag-only changes. The archive bytes below are deliberate
/// sentinels: this verifies successful cache-hit/disabled policy selection, not
/// archive validity or a successful bake. The real build script runs in a child
/// of a dependency-free Cargo fixture; its empty PATH forbids a nested baker.
#[test]
fn cargo_rechecks_auto_precompile_policy_on_environment_changes() {
    const CHILD: &str = "VERUM_CACHE_POLICY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        build_script::main();
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let manifest = root.path().join("crates/verum_compiler/Cargo.toml");
    let target = root.path().join("target");
    write(root.path(), "core/mod.vr", CORE[0].1);
    write(root.path(), PARSER_DECLARATIONS, PARSER_SOURCE);
    // An absent rerun-if-changed input makes every Cargo invocation dirty and
    // would hide the missing environment dependency. Materialize the complete
    // reported roster before computing the fixture's stable cache key.
    for path in key(root.path(), build_script::PRECOMPILE_SCHEMA_VERSION, CORE).1 {
        if !path.exists() {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"").unwrap();
        }
    }
    let expected_key = key(root.path(), build_script::PRECOMPILE_SCHEMA_VERSION, CORE).0;
    let cache = target.join("precompiled-stdlib");
    fs::create_dir_all(&cache).unwrap();
    for artifact in [
        "runtime.vbca",
        "runtime.core_metadata",
        "runtime.symbol_graph",
    ] {
        fs::write(
            cache.join(artifact),
            b"policy-control sentinel; not an archive",
        )
        .unwrap();
    }
    fs::write(
        cache.join("runtime.vbca.checksum"),
        expected_key.to_hex().as_str(),
    )
    .unwrap();
    fs::write(
        cache.join("runtime.vbca.schema"),
        build_script::PRECOMPILE_SCHEMA_VERSION,
    )
    .unwrap();
    write(root.path(), "crates/verum_compiler/Cargo.toml", b"[package]\nname = \"stdlib-policy-probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n");
    write(
        root.path(),
        "crates/verum_compiler/Cargo.lock",
        b"version = 4\n\n[[package]]\nname = \"stdlib-policy-probe\"\nversion = \"0.0.0\"\n",
    );
    write(
        root.path(),
        "crates/verum_compiler/src/lib.rs",
        b"pub const PROBE: bool = true;\n",
    );
    // Only the wrapper is synthetic. It delegates to the already compiled
    // production entry point and forwards its exact Cargo dependency output.
    let wrapper = format!(
        r#"
use std::io::Write;
fn main() {{
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let output = std::process::Command::new({exe:?})
        .args(["--exact", "cargo_rechecks_auto_precompile_policy_on_environment_changes", "--nocapture"])
        .env("VERUM_CACHE_POLICY_CHILD", "1")
        .env("PATH", {empty_path:?})
        .output().expect("run actual build script");
    let mut counter = std::fs::OpenOptions::new().create(true).append(true)
        .open(out.join("policy-invocations.log")).unwrap();
    counter.write_all(b"run\n").unwrap();
    std::fs::write(out.join("policy-last.stdout"), &output.stdout).unwrap();
    std::fs::write(out.join("policy-last.stderr"), &output.stderr).unwrap();
    print!("{{}}", String::from_utf8_lossy(&output.stdout));
    eprint!("{{}}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "production build-script child failed");
}}
"#,
        exe = std::env::current_exe().unwrap(),
        empty_path = root.path().join("no-executables")
    );
    write(root.path(), "crates/verum_compiler/build.rs", wrapper);
    let evidence = std::env::var_os("VERUM_CACHE_POLICY_EVIDENCE_DIR").map(PathBuf::from);
    if let Some(directory) = &evidence {
        fs::create_dir(directory).expect("policy evidence directory must be new");
    }
    let mut states: List<(Option<&str>, Option<&str>)> = List::from_iter([(None, None)]);
    for flag in ["VERUM_NO_AUTO_PRECOMPILE", "DOCS_RS"] {
        states.extend([
            (Some(flag), Some("1")),
            (Some(flag), Some("0")),
            (None, None),
        ]);
    }
    let mut expected_runs = 0usize;
    let mut last_out = None;
    for (step, (flag, value)) in states.into_iter().enumerate() {
        // Repeat every state without changing source, output paths or flags.
        // The first call must rerun; the second must reuse the same fingerprint.
        expected_runs += 1;
        for repeat in 0..2 {
            let label = format!("{step:02}-{repeat}-{}", flag.unwrap_or("automatic"));
            let stdout_path = root.path().join(format!("{label}.stdout"));
            let stderr_path = root.path().join(format!("{label}.stderr"));
            let mut command = Command::new(env!("CARGO"));
            command
                .args([
                    "build",
                    "--offline",
                    "--locked",
                    "--message-format=json",
                    "-vv",
                ])
                .arg("--manifest-path")
                .arg(&manifest)
                .arg("--target-dir")
                .arg(&target)
                .current_dir(manifest.parent().unwrap())
                .env_remove("VERUM_NO_AUTO_PRECOMPILE")
                .env_remove("DOCS_RS")
                .env_remove("VERUM_ALLOW_STALE_STDLIB")
                .env("CARGO_LOG", "cargo::core::compiler::fingerprint=trace")
                .stdout(Stdio::from(fs::File::create(&stdout_path).unwrap()))
                .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
            if let (Some(flag), Some(value)) = (flag, value) {
                command.env(flag, value);
            }
            let mut child = OwnedChild(command.spawn().unwrap());
            let deadline = Instant::now() + Duration::from_secs(45);
            let status = loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    break status;
                }
                assert!(
                    Instant::now() < deadline,
                    "{label}: Cargo policy control exceeded 45 seconds"
                );
                std::thread::sleep(Duration::from_millis(10));
            };
            let stdout: Text = fs::read_to_string(&stdout_path).unwrap().into();
            let stderr: Text = fs::read_to_string(&stderr_path).unwrap().into();
            if let Some(directory) = &evidence {
                fs::copy(&stdout_path, directory.join(format!("{label}.stdout"))).unwrap();
                fs::copy(&stderr_path, directory.join(format!("{label}.stderr"))).unwrap();
            }
            assert!(status.success(), "{label}: {status}; {stdout}; {stderr}");
            for line in stdout.lines() {
                if let Ok(message) = serde_json::from_str::<serde_json::Value>(line.as_str())
                    && message["reason"] == "build-script-executed"
                {
                    last_out = Some(PathBuf::from(
                        message["out_dir"].as_str().expect("script output path"),
                    ));
                }
            }
            let output = last_out
                .as_ref()
                .expect("Cargo reported actual build-script execution");
            let runs = fs::read_to_string(output.join("policy-invocations.log"))
                .unwrap()
                .lines()
                .count();
            assert_eq!(
                runs, expected_runs,
                "{label}: expected flag change to rerun once and identical state to stay fresh; {stderr}"
            );
            let policy: Text = fs::read_to_string(output.join("policy-last.stdout"))
                .unwrap()
                .into();
            assert_eq!(
                policy.contains("Stdlib precompile cache HIT"),
                flag.is_none(),
                "{label}: {policy}"
            );
            assert!(
                !policy.contains("Refreshing stdlib precompile"),
                "sentinel cache must prevent baking: {policy}"
            );
            assert_eq!(
                expected_key,
                key(root.path(), build_script::PRECOMPILE_SCHEMA_VERSION, CORE).0,
                "the source/checksum inputs must stay unchanged across environment transitions"
            );
            assert_eq!(
                fs::read_to_string(cache.join("runtime.vbca.checksum")).unwrap(),
                expected_key.to_hex().as_str()
            );
            assert_eq!(
                fs::read(cache.join("runtime.vbca")).unwrap(),
                b"policy-control sentinel; not an archive"
            );
            assert!(
                !target.join("precompile-bootstrap").exists(),
                "no nested baker target may be created"
            );
            eprintln!(
                "policy control {label}: runs={runs}, automatic={}",
                flag.is_none()
            );
            if let Some(directory) = &evidence {
                fs::write(
                    directory.join(format!("{label}.build-script.stdout")),
                    policy.as_bytes(),
                )
                .unwrap();
            }
        }
    }
}
