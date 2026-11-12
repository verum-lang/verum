//! Automatic archive identity uses the actual build-script hash implementation.
//! Policy controls include a dependency-free Cargo fixture; none bake an archive.

#[allow(dead_code)]
#[path = "../build.rs"]
mod build_script;
#[path = "../build_support/stdlib_cache.rs"]
mod stdlib_cache;
#[allow(dead_code)]
#[path = "../build_support/bake_receipt.rs"]
mod bake_receipt;

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
fn dereference_copy_semantics_invalidate_the_archive_without_schema_changes() {
    const PATH: &str = "crates/verum_vbc/src/codegen/place_copies.rs";
    const SOURCE: &str = include_str!("../../verum_vbc/src/codegen/place_copies.rs");
    let root = tempfile::tempdir().unwrap();
    let with_copy = "TypeRef::Reference { inner, .. } => Some(*inner)";
    let without_copy = "TypeRef::Reference { .. } => None";
    assert_eq!(SOURCE.matches(with_copy).count(), 1);
    write(root.path(), PATH, SOURCE.replace(with_copy, without_copy));
    let before = key(root.path(), SCHEMA, CORE).0;
    write(root.path(), PATH, SOURCE);
    let (after, dependencies) = key(root.path(), SCHEMA, CORE);
    assert_ne!(before, after, "dereference-copy semantics are an archive producer input");
    assert_eq!(after, key(root.path(), SCHEMA, CORE).0);
    assert_eq!(dependencies.iter().filter(|path| **path == root.path().join(PATH)).count(), 1);
}

#[test]
fn forward_declaration_witnesses_invalidate_the_archive_without_schema_changes() {
    const PATH: &str = "crates/verum_vbc/src/codegen/declaration_interface.rs";
    const SOURCE: &str = include_str!("../../verum_vbc/src/codegen/declaration_interface.rs");
    let root = tempfile::tempdir().unwrap();
    let preserved = "descriptor.semantic_params = Some(semantic.clone());";
    assert_eq!(SOURCE.matches(preserved).count(), 1);
    write(root.path(), PATH, SOURCE.replace(preserved, "descriptor.semantic_params = None;"));
    let before = key(root.path(), SCHEMA, CORE).0;
    write(root.path(), PATH, SOURCE);
    let (after, dependencies) = key(root.path(), SCHEMA, CORE);
    assert_ne!(before, after, "forward ownership witnesses are an archive producer input");
    assert_eq!(after, key(root.path(), SCHEMA, CORE).0);
    assert_eq!(dependencies.iter().filter(|path| **path == root.path().join(PATH)).count(), 1);
}

#[test]
fn ast_and_lowering_source_trees_invalidate_on_edits_additions_and_removals() {
    for (tree, file) in [
        ("crates/verum_ast/src", "visitor.rs"),
        ("crates/verum_vbc/src/codegen", "registers.rs"),
        ("crates/verum_vbc/src/codegen", "nested/new_lowering.rs"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (missing, watched) = key(root.path(), SCHEMA, CORE);
        assert!(watched.contains(&root.path().join(tree)), "Cargo must detect future files in {tree}");
        let source = &format!("{tree}/{file}");
        write(root.path(), source, b"original");
        let (original, dependencies) = key(root.path(), SCHEMA, CORE);
        assert_ne!(original, missing, "adding {source}");
        assert_eq!(dependencies.iter().filter(|path| **path == root.path().join(source)).count(), 1);
        write(root.path(), source, b"changed");
        assert_ne!(key(root.path(), SCHEMA, CORE).0, original, "editing {source}");
        fs::remove_file(root.path().join(source)).unwrap();
        assert_eq!(key(root.path(), SCHEMA, CORE).0, missing, "removing {source}");
    }
}

#[test]
fn producer_content_cannot_impersonate_a_second_source_file() {
    let root = tempfile::tempdir().unwrap();
    let first = "crates/verum_ast/src/a.rs";
    let second = "crates/verum_ast/src/b.rs";
    write(root.path(), first, format!("first\0{second}\0last"));
    let single = key(root.path(), SCHEMA, CORE).0;
    write(root.path(), first, "first");
    write(root.path(), second, "last");
    assert_ne!(single, key(root.path(), SCHEMA, CORE).0);
}

#[test]
fn selection_ownership_semantics_invalidate_the_archive_without_schema_changes() {
    const PATH: &str = "crates/verum_vbc/src/codegen/select.rs";
    const SOURCE: &str = include_str!("../../verum_vbc/src/codegen/select.rs");
    let root = tempfile::tempdir().unwrap();
    let owned = "let owned = Self::future_is_owned(inner);";
    assert_eq!(SOURCE.matches(owned).count(), 1);
    write(root.path(), PATH, SOURCE.replace(owned, "let owned = false;"));
    let before = key(root.path(), SCHEMA, CORE).0;
    write(root.path(), PATH, SOURCE);
    let (after, dependencies) = key(root.path(), SCHEMA, CORE);
    assert_ne!(before, after, "selection ownership changes the baked program");
    assert_eq!(dependencies.iter().filter(|path| **path == root.path().join(PATH)).count(), 1);
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
    for dependency in &dependencies {
        assert!(dependency.exists(), "obsolete producer input keeps Cargo dirty: {}", dependency.display());
    }
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
            .env("TARGET", env!("VERUM_BAKE_TARGET"))
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
        for policy in ["VERUM_NO_AUTO_PRECOMPILE", "DOCS_RS", "VERUM_PRECOMPILE_THREADS"] {
            let dependency = format!("cargo:rerun-if-env-changed={policy}");
            assert!(stdout.lines().iter().any(|line| line.as_str() == dependency),
                "{flag}={value}: {stdout}");
        }
        let publication = format!(
            "cargo:rerun-if-changed={}",
            target.join("precompiled-stdlib").display()
        );
        assert!(
            stdout
                .lines()
                .iter()
                .any(|line| line.as_str() == publication.as_str())
        );
        assert!(!stdout.contains("Refreshing stdlib precompile"));
        assert!(!stdout.contains("Stdlib precompile cache HIT"));
        assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
        assert_eq!(fs::read_dir(target.join("precompiled-stdlib")).unwrap().count(), 0,
            "disabled precompile may establish the watched directory, but must not bake or publish artifacts");
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
    write(root.path(), "crates/verum_ast/src/visitor.rs", b"// fixture AST traversal");
    write(root.path(), "crates/verum_vbc/src/codegen/mod.rs", b"// fixture lowering");
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
    let producer = stdlib_cache::producer_hasher(root.path(), build_script::PRECOMPILE_SCHEMA_VERSION, |_| {}).finalize().to_hex();
    let inputs = bake_receipt::Inputs {
        producer: producer.as_str(),
        target: env!("VERUM_BAKE_TARGET"),
        source: bake_receipt::source_digests(CORE.iter().copied()),
    };
    inputs.record(&cache.join("runtime.vbca"), blake3::hash(b"policy-control sentinel; not an archive")).unwrap();
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
    #[derive(Clone, Copy, PartialEq)]
    enum CacheState { Present, Absent, WithoutChecksum, WithoutReceipt }
    let cache_files: List<_> = fs::read_dir(&cache).unwrap().map(|entry| {
        let entry = entry.unwrap();
        (entry.file_name(), fs::read(entry.path()).unwrap())
    }).collect();
    let mut states = List::from_iter([(None, None, CacheState::Present)]);
    for flag in ["VERUM_NO_AUTO_PRECOMPILE", "DOCS_RS"] {
        states.extend([
            (Some(flag), Some("1"), CacheState::Present),
            (Some(flag), Some("0"), CacheState::Present),
            (None, None, CacheState::Present),
        ]);
    }
    // Keep the environment fixed while cache membership changes. A disabled
    // bake must stay fresh with no cache, yet notice publication and removal.
    for state in [CacheState::Absent, CacheState::Present,
        CacheState::WithoutChecksum, CacheState::Present,
        CacheState::WithoutReceipt, CacheState::Present] {
        states.push((Some("VERUM_NO_AUTO_PRECOMPILE"), Some("1"), state));
    }
    let mut expected_runs = 0usize;
    let mut last_out = None;
    let mut previous_cache = CacheState::Present;
    for (step, (flag, value, cache_state)) in states.into_iter().enumerate() {
        if cache_state != previous_cache {
            if cache.exists() { fs::remove_dir_all(&cache).unwrap(); }
            if cache_state != CacheState::Absent {
                fs::create_dir_all(&cache).unwrap();
                for (name, bytes) in &cache_files { fs::write(cache.join(name), bytes).unwrap(); }
                match cache_state {
                    CacheState::WithoutChecksum => fs::remove_file(cache.join("runtime.vbca.checksum")).unwrap(),
                    CacheState::WithoutReceipt => fs::remove_file(bake_receipt::path(&cache.join("runtime.vbca"))).unwrap(),
                    _ => {},
                }
            }
            previous_cache = cache_state;
        }
        // Repeat every state without changing source, output paths or flags.
        // Creating the missing publication directory happens after Cargo's
        // build-start timestamp, so Cargo conservatively rechecks it once.
        // That initialization must settle; a missing-file dependency would
        // instead rerun forever. Existing directories settle on the first run.
        expected_runs += 1;
        let repeats = if cache_state == CacheState::Absent { 3 } else { 2 };
        for repeat in 0..repeats {
            if cache_state == CacheState::Absent && repeat == 1 { expected_runs += 1; }
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
                // The production script gives this absolute environment
                // value precedence over OUT_DIR when locating its cache.
                .env("CARGO_TARGET_DIR", &target)
                .env("CARGO_BUILD_JOBS", "1")
                .env_remove("CARGO_MAKEFLAGS")
                .env_remove("MAKEFLAGS")
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
            let policy: Text = fs::read_to_string(output.join("policy-last.stdout"))
                .unwrap()
                .into();
            if let Some(directory) = &evidence {
                fs::write(directory.join(format!("{label}.build-script.stdout")), policy.as_bytes()).unwrap();
            }
            assert_eq!(
                runs, expected_runs,
                "{label}: expected changed inputs to rerun once and identical state to stay fresh; {stderr}"
            );
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
            if cache_state == CacheState::Present {
                assert_eq!(fs::read_to_string(cache.join("runtime.vbca.checksum")).unwrap(), expected_key.to_hex().as_str());
                assert_eq!(fs::read(cache.join("runtime.vbca")).unwrap(), b"policy-control sentinel; not an archive");
            }
            assert_eq!(policy.contains("Source-only stdlib bootstrap"), cache_state != CacheState::Present, "{label}: {policy}");
            assert!(
                !target.join("precompile-bootstrap").exists(),
                "no nested baker target may be created"
            );
            for (private, cached) in [
                ("stdlib_runtime.vbca", "runtime.vbca"),
                ("stdlib_runtime.core_metadata", "runtime.core_metadata"),
                ("stdlib_runtime.symbol_graph", "runtime.symbol_graph"),
            ] {
                if cache_state == CacheState::Present {
                    assert_eq!(fs::read(output.join(private)).unwrap(), fs::read(cache.join(cached)).unwrap());
                } else {
                    assert!(fs::read(output.join(private)).unwrap().is_empty(), "stale {private} survived");
                }
            }
            eprintln!(
                "policy control {label}: runs={runs}, automatic={}",
                flag.is_none()
            );
        }
    }

    // A present but unrelated index must force a refresh. With the baker
    // unavailable, the ordinary build must fail before any bytes can be embedded.
    fs::write(cache.join("runtime.core_metadata"), b"another producer's metadata").unwrap();
    let output = last_out.unwrap();
    let build = |bootstrap: bool| {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "cargo_rechecks_auto_precompile_policy_on_environment_changes", "--nocapture"])
            .env(CHILD, "1").env_remove("VERUM_NO_AUTO_PRECOMPILE")
            .env_remove("DOCS_RS").env_remove("VERUM_ALLOW_STALE_STDLIB")
            .env("TARGET", env!("VERUM_BAKE_TARGET"))
            .env("CARGO_MANIFEST_DIR", manifest.parent().unwrap()).env("CARGO_TARGET_DIR", &target)
            .env("OUT_DIR", &output).env("PATH", root.path().join("no-executables"))
            .current_dir(root.path());
        if bootstrap { command.env("VERUM_NO_AUTO_PRECOMPILE", "1"); }
        command.output().unwrap()
    };
    let bootstrap = build(true);
    assert!(bootstrap.status.success());
    assert!(String::from_utf8(bootstrap.stdout).unwrap().contains("Source-only stdlib bootstrap"));
    for file in ["stdlib_runtime.vbca", "stdlib_runtime.core_metadata", "stdlib_runtime.symbol_graph"] {
        assert!(fs::read(output.join(file)).unwrap().is_empty(), "stale {file} survived");
    }
    assert_eq!(fs::read(cache.join("runtime.core_metadata")).unwrap(), b"another producer's metadata");
    let rejected = build(false);
    assert!(!rejected.status.success());
    let log = String::from_utf8(rejected.stdout).unwrap();
    assert!(log.contains("Refreshing stdlib precompile artefacts"), "{log}");
    assert!(!log.contains("Stdlib precompile cache HIT"), "{log}");
}

#[test]
fn receipt_requires_the_producer_target_and_unambiguous_source_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("runtime.vbca");
    fs::write(&archive, b"archive bytes").unwrap();
    let inputs = |producer, target, merged| {
        let source = if merged {
            bake_receipt::source_digests([("a.vr", b"x\0b.vr\0y".as_slice())])
        } else {
            bake_receipt::source_digests([("a.vr", b"x".as_slice()), ("b.vr", b"y".as_slice())])
        };
        bake_receipt::Inputs {
            source,
            producer,
            target,
        }
    };
    let original = inputs("producer-a", "first-target", false);
    fs::write(archive.with_extension("core_metadata"), b"stable metadata").unwrap();
    fs::write(archive.with_extension("symbol_graph"), b"stable graph").unwrap();
    original.record(&archive, blake3::hash(b"archive bytes")).unwrap();
    original.verify_file(&archive).unwrap();
    let merged = inputs("producer-a", "first-target", true);
    assert_eq!(original.source.sdk, merged.source.sdk);
    for mismatch in [
        merged,
        inputs("producer-b", "first-target", false),
        inputs("producer-a", "second-target", false),
    ] {
        assert!(mismatch.verify_file(&archive).is_err());
    }
}

#[test]
fn sidecar_replacement_cannot_pass_cache_or_embedding_verification() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("runtime.vbca");
    let copy = root.path().join("embedded.vbca");
    let inputs = bake_receipt::Inputs {
        producer: "producer",
        target: "target",
        source: bake_receipt::source_digests(CORE.iter().copied()),
    };
    for path in [&source, &copy] {
        fs::write(path, b"bytecode").unwrap();
        fs::write(path.with_extension("core_metadata"), b"metadata").unwrap();
        fs::write(path.with_extension("symbol_graph"), b"graph").unwrap();
    }
    inputs.record(&source, blake3::hash(b"bytecode")).unwrap();
    inputs.verify_copy(&source, &copy).unwrap();
    for (ext, valid) in [("core_metadata", b"metadata".as_slice()), ("symbol_graph", b"graph")] {
        for path in [&source, &copy] {
            fs::write(path.with_extension(ext), b"from a different build").unwrap();
            assert!(inputs.verify_copy(&source, path).is_err());
            fs::write(path.with_extension(ext), b"").unwrap();
            assert!(inputs.verify_copy(&source, path).is_err());
            fs::write(path.with_extension(ext), valid).unwrap();
        }
    }
    let receipt_path = bake_receipt::path(&source);
    let v2 = fs::read_to_string(&receipt_path).unwrap();
    let v1 = v2.split("metadata=").next().unwrap().replace("bake-v2", "bake-v1");
    fs::write(&receipt_path, v1).unwrap();
    assert!(inputs.verify_file(&source).is_err(), "unbound legacy sidecars");
}
