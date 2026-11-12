//! T2279: explicit precompile paths are configuration authority outside a workspace.
use super::resolve_config;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use verum_compiler::precompile::PrecompileConfig;

#[test]
fn explicit_paths_and_options_are_preserved() {
    let source = PathBuf::from("relative-source/core");
    let output = PathBuf::from("chosen/result.vbca");
    let cfg = resolve_config(
        Some(source.clone()),
        Some(output.clone()),
        Some("target-contract".into()),
        std::num::NonZeroUsize::new(3),
        true,
    )
    .unwrap();
    assert_eq!(cfg.stdlib_path, source);
    assert_eq!(cfg.output_path, output);
    assert_eq!(cfg.target_triple.as_deref(), Some("target-contract"));
    assert!(cfg.verbose);
    assert_eq!(cfg.threads.map(|n| n.get()), Some(3));
}

#[test]
fn explicit_source_default_uses_the_shared_constructor() {
    let source = PathBuf::from("external-sdk/core");
    let cfg = resolve_config(Some(source.clone()), None, None, None, false).unwrap();
    let common = PrecompileConfig::for_source(source.clone(), Path::new("."));
    assert_eq!(cfg.stdlib_path, source);
    assert_eq!(cfg.output_path, common.output_path);
    assert!(cfg.target_triple.is_none());
    assert!(!cfg.verbose);
    assert!(cfg.threads.is_none());
}

#[test]
fn workspace_defaults_use_the_same_archive_location() {
    let root = TempDir::new().unwrap();
    std::fs::create_dir_all(root.path().join("core")).unwrap();
    std::fs::write(root.path().join("core/mod.vr"), "module core;\n").unwrap();
    let nested = root.path().join("nested/deeper");
    std::fs::create_dir_all(&nested).unwrap();
    let canonical = root.path().canonicalize().unwrap();
    let inferred = PrecompileConfig::for_workspace(&nested).unwrap();
    let explicit = PrecompileConfig::for_source(canonical.join("core"), &canonical);
    assert_eq!(inferred.stdlib_path, explicit.stdlib_path);
    assert_eq!(inferred.output_path, explicit.output_path);
    assert_eq!(
        inferred.output_path,
        canonical.join("target/precompiled-stdlib/runtime.vbca")
    );
}

#[test]
fn explicit_paths_work_from_an_unrelated_cwd() {
    child_case("explicit");
}
#[test]
fn source_discovery_still_requires_a_workspace() {
    child_case("missing");
}
#[test]
fn output_override_does_not_supply_a_missing_source() {
    child_case("output_only");
}
#[test]
fn omitted_source_retains_workspace_default_and_output_override() {
    child_case("workspace");
}

fn child_case(case: &str) {
    let root = TempDir::new().unwrap();
    let module = module_path!().split_once("::").unwrap().1;
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("{module}::isolated_paths_child"),
            "--nocapture",
        ])
        .env("VERUM_T2279_CASE", case)
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "case={case}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("T2279_CHILD_ASSERTED"));
}

#[test]
fn isolated_paths_child() {
    let Ok(case) = std::env::var("VERUM_T2279_CASE") else {
        return;
    };
    let cwd = std::env::current_dir().unwrap();
    assert!(PrecompileConfig::for_workspace(&cwd).is_err());
    match case.as_str() {
        "explicit" => {
            let source = cwd.join("not-yet-created-sdk/core");
            let output = cwd.join("archive/result.vbca");
            let cfg =
                resolve_config(Some(source.clone()), Some(output.clone()), None, None, false).unwrap();
            assert_eq!(cfg.stdlib_path, source);
            assert_eq!(cfg.output_path, output);
            // Source existence is validated by the real producer, not workspace discovery.
            let default = resolve_config(Some(source.clone()), None, None, None, false).unwrap();
            assert_eq!(default.stdlib_path, source);
            assert_eq!(
                default.output_path,
                PrecompileConfig::for_source(PathBuf::new(), Path::new(".")).output_path
            );
        }
        "missing" | "output_only" => {
            let out = (case == "output_only").then(|| cwd.join("output.vbca"));
            let error = resolve_config(None, out, None, None, false)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("failed to resolve workspace root from cwd"),
                "{error}"
            );
            assert!(error.contains("Pass --stdlib-path explicitly"), "{error}");
        }
        "workspace" => {
            std::fs::create_dir(cwd.join("core")).unwrap();
            std::fs::write(cwd.join("core/mod.vr"), "module core;\n").unwrap();
            let canonical = cwd.canonicalize().unwrap();
            let default = resolve_config(None, None, None, None, false).unwrap();
            assert_eq!(default.stdlib_path, canonical.join("core"));
            assert_eq!(
                default.output_path,
                canonical.join("target/precompiled-stdlib/runtime.vbca")
            );
            let output = cwd.join("chosen.vbca");
            let overridden = resolve_config(None, Some(output.clone()), None, None, false).unwrap();
            assert_eq!(overridden.stdlib_path, canonical.join("core"));
            assert_eq!(overridden.output_path, output);
        }
        other => panic!("unknown child case: {other}"),
    }
    println!("T2279_CHILD_ASSERTED");
}
