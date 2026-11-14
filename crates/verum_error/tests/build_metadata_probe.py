#!/usr/bin/env python3
"""Exercise the real CLI identity producer and crash API in a small Cargo graph.

The executable uses the actual verum_error crate and the CLI's unchanged build
script/identity module. One small consumer checks transitive Cargo freshness;
this does not build or claim execution coverage for verum_types, CLI, LLVM or AOT.
All fixture repositories and evidence are local. The target must already exist.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
BUILD = ROOT / "crates/verum_cli/build.rs"
IDENTITY = ROOT / "crates/verum_cli/src/build_identity.rs"
APP = "verum_build_metadata_fixture"
CONSUMER = "verum_metadata_consumer"
MAIN = '''mod build_identity;
use verum_error::crash::{self, CrashReporterConfig};
fn main() {
    crash::install(CrashReporterConfig {
        app_name: "identity-probe".into(), app_version: "fixture".into(),
        build: build_identity::BUILD_IDENTITY,
        report_dir: Some(std::env::var_os("VERUM_PROBE_REPORT_DIR").unwrap().into()),
        capture_backtrace: false, install_signal_handlers: false,
        ..Default::default()
    });
    let _ = std::panic::catch_unwind(|| panic!("artifact identity probe"));
    let value = crash::env_snapshot().unwrap();
    println!("CAPTURED_SHA={}", value.build_git_sha);
    println!("CAPTURED_DIRTY={}", value.build_git_dirty);
    println!("CAPTURED_TIMESTAMP={}", value.build_timestamp);
    println!("CAPTURED_TARGET={}", value.build_target);
    println!("CAPTURED_HOST={}", value.build_host);
    println!("CAPTURED_RUSTC={}", value.build_rustc);
    println!("SOURCE_MARKER={}", verum_metadata_consumer::marker());
}
'''
CONSUMER_SOURCE = '''pub fn marker() -> &'static str {
    let _identity = verum_error::crash::BuildIdentity::default();
    "initial"
}
'''


def digest(data):
    return hashlib.sha256(data).hexdigest()


def run_probe(output, target):
    if not target.is_dir():
        raise RuntimeError("reuse an existing session-private target directory")
    output.mkdir()  # Never overwrite evidence.
    ordinary = output / "ordinary"
    app = ordinary / "crates/application"
    consumer = ordinary / "crates/consumer"
    for package in [app, consumer]:
        (package / "src").mkdir(parents=True)
    (ordinary / "Cargo.toml").write_text('''[workspace]
resolver = "2"
members = ["crates/application", "crates/consumer"]
[profile.dev]
opt-level = 0
debug = 1
incremental = true
split-debuginfo = "packed"
codegen-units = 16
[profile.dev.package."*"]
opt-level = 2
''')
    error_path = json.dumps(str(ROOT / "crates/verum_error"))
    (app / "Cargo.toml").write_text(
        f'[package]\nname="{APP}"\nversion="0.0.0"\nedition="2024"\n'
        f'[dependencies]\nverum_error={{path={error_path}}}\n'
        f'{CONSUMER}={{path="../consumer"}}\n')
    (consumer / "Cargo.toml").write_text(
        f'[package]\nname="{CONSUMER}"\nversion="0.0.0"\nedition="2024"\n'
        f'[dependencies]\nverum_error={{path={error_path}}}\n')
    (app / "build.rs").write_bytes(BUILD.read_bytes())
    (app / "src/build_identity.rs").write_bytes(IDENTITY.read_bytes())
    (app / "src/main.rs").write_text(MAIN)
    (consumer / "src/lib.rs").write_text(CONSUMER_SOURCE)
    (ordinary / ".gitignore").write_text("/target/\n")
    whitespace_path = ordinary / " leading source"
    whitespace_path.write_text("initial\n")
    (ordinary / "rust-toolchain.toml").write_bytes((ROOT / "rust-toolchain.toml").read_bytes())
    (ordinary / "Cargo.lock").write_bytes((ROOT / "Cargo.lock").read_bytes())
    logs = output / "logs"
    logs.mkdir()
    hooks = output / "empty-hooks"
    hooks.mkdir()
    environment = {**os.environ, "CARGO_TARGET_DIR": str(target),
                   "CARGO_BUILD_JOBS": "2", "VERUM_NO_AUTO_PRECOMPILE": "1",
                   "CARGO_TERM_COLOR": "never", "GIT_OPTIONAL_LOCKS": "0"}
    environment.pop("SOURCE_DATE_EPOCH", None)
    source_paths = [BUILD, IDENTITY, ROOT / "crates/verum_error/src/crash.rs"]
    sources = {str(path): digest(path.read_bytes()) for path in source_paths}
    assert not (ROOT / "crates/verum_error/build.rs").exists()

    def command(args, cwd=ordinary, check=True, extra_env=None):
        result = subprocess.run(args, cwd=cwd, env={**environment, **(extra_env or {})},
                                capture_output=True, text=True, timeout=900)
        if check and result.returncode:
            raise RuntimeError(f"{args!r} failed: {result.stderr[-8000:]}")
        return result

    def git(*args, cwd=ordinary, check=True):
        return command(["git", *args], cwd, check)

    git("init")
    git("config", "user.name", "Verum Build Fixture")
    git("config", "user.email", "fixture@example.invalid")
    git("config", "commit.gpgsign", "false")
    git("config", "gc.auto", "0")
    git("config", "core.hooksPath", str(hooks))
    command(["cargo", "metadata", "--offline", "--format-version=1"])
    git("add", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".gitignore", " leading source", "crates")
    git("commit", "--no-gpg-sign", "-m", "Identity fixture")
    rustc = command([environment.get("RUSTC", "rustc"), "-vV"]).stdout
    host = next(line.removeprefix("host: ") for line in rustc.splitlines() if line.startswith("host: "))
    observations = []

    def probe(name, cwd, previous=None, marker="initial", ref_only=False):
        print(f"running {name}", flush=True)
        revision = git("rev-parse", "--short=12", "HEAD", cwd=cwd, check=False)
        status = git("status", "--porcelain", cwd=cwd, check=False)
        sha = revision.stdout.strip() if revision.returncode == 0 else "unknown"
        dirty = ("dirty" if status.stdout.strip() else "clean") if status.returncode == 0 else "unknown"
        build = command(["cargo", "build", "--offline", "--locked", "-p", APP,
                         "--message-format=json"], cwd, check=False)
        (logs / f"{name}.cargo.jsonl").write_text(build.stdout)
        (logs / f"{name}.stderr.log").write_text(build.stderr)
        if build.returncode:
            raise RuntimeError(f"{name}: cargo failed; see {logs}")
        artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith('{')]
        artifacts = [item for item in artifacts if item.get("reason") == "compiler-artifact"]
        fresh = {item["target"]["name"]: item["fresh"] for item in artifacts}
        executable = Path(next(item["executable"] for item in artifacts
                               if item["target"]["name"] == APP and item.get("executable")))
        identity = {"sha256": digest(executable.read_bytes()), "mtime_ns": executable.stat().st_mtime_ns}
        reports = output / "reports" / name
        execution = command([str(executable)], cwd, extra_env={
            "VERUM_PROBE_REPORT_DIR": str(reports), "VERUM_BUILD_GIT_SHA": "runtime-spoof"})
        (logs / f"{name}.stdout.log").write_text(execution.stdout)
        (logs / f"{name}.crash.log").write_text(execution.stderr)
        captured = dict(line.split("=", 1) for line in execution.stdout.splitlines()
                        if line.startswith(("CAPTURED_", "SOURCE_MARKER=")))
        report_files = list(reports.glob("*.json"))
        if len(report_files) != 1:
            raise RuntimeError(f"{name}: expected one actual crash report")
        report = json.loads(report_files[0].read_text())["environment"]
        checks = {
            "git_sha": captured.get("CAPTURED_SHA") == sha,
            "git_dirty": captured.get("CAPTURED_DIRTY") == dirty,
            "source_marker": captured.get("SOURCE_MARKER") == marker,
            "target": captured.get("CAPTURED_TARGET") == host,
            "host": captured.get("CAPTURED_HOST") == host,
            "rustc": captured.get("CAPTURED_RUSTC") == rustc.splitlines()[0],
            "crash_identity": report["build_git_sha"] == sha and report["build_git_dirty"] == dirty
                and report["build_target"] == host and report["build_timestamp"] == captured.get("CAPTURED_TIMESTAMP"),
            "actual_cli_build_script": (cwd / "crates/application/build.rs").read_bytes() == BUILD.read_bytes(),
            "actual_cli_identity_module": (cwd / "crates/application/src/build_identity.rs").read_bytes() == IDENTITY.read_bytes(),
        }
        if previous:
            checks.update({"unchanged_executable": identity == previous["executable"],
                           "unchanged_timestamp": captured.get("CAPTURED_TIMESTAMP") == previous["captured"].get("CAPTURED_TIMESTAMP"),
                           "executable_fresh": fresh[APP]})
        if ref_only:
            checks.update({"actual_error_fresh": fresh.get("verum_error") is True,
                           "actual_common_fresh": fresh.get("verum_common") is True,
                           "transitive_consumer_fresh": fresh.get(CONSUMER) is True,
                           "executable_rebuilt": fresh[APP] is False})
        record = {"name": name, "cwd": str(cwd), "expected_sha": sha, "expected_dirty": dirty,
                  "captured": captured, "fresh": fresh, "executable": identity, "checks": checks}
        observations.append(record)
        print(f"{name}: {'passed' if all(checks.values()) else 'FAILED'}", flush=True)
        return record

    first = probe("ordinary-first", ordinary)
    time.sleep(1.1)
    probe("ordinary-identical", ordinary, first)
    git("commit", "--no-gpg-sign", "--allow-empty", "-m", "Advance only HEAD")
    probe("ordinary-ref-advance", ordinary, ref_only=True)
    git("pack-refs", "--all", "--prune")
    first = probe("packed-first", ordinary, ref_only=True)
    probe("packed-identical", ordinary, first)
    git("commit", "--no-gpg-sign", "--allow-empty", "-m", "Create loose ref from packed")
    probe("packed-to-loose", ordinary, ref_only=True)
    # This name sorts before other tracked paths. Trimming the NUL roster loses
    # its initial space and silently misses the metadata-only source edit.
    whitespace_path.write_text("changed\n")
    probe("whitespace-path-dirty", ordinary, ref_only=True)
    whitespace_path.write_text("initial\n")
    probe("whitespace-path-clean", ordinary, ref_only=True)
    # The file did not exist when the previous watch list was produced.
    untracked = app / "src/new_untracked_source.rs"
    untracked.write_text("// newly created source input\n")
    probe("untracked-source-dirty", ordinary, ref_only=True)
    untracked.unlink()
    probe("untracked-source-removed", ordinary, ref_only=True)
    library = consumer / "src/lib.rs"
    library.write_text(CONSUMER_SOURCE.replace('"initial"', '"changed"'))
    probe("dependency-source-dirty", ordinary, marker="changed")
    git("add", "crates/consumer/src/lib.rs")
    probe("dependency-source-staged", ordinary, marker="changed", ref_only=True)
    git("commit", "--no-gpg-sign", "-m", "Change consumer source")
    probe("dependency-source-clean", ordinary, marker="changed", ref_only=True)
    # Cargo trims trailing whitespace in watch directives. Refuse that path
    # explicitly instead of emitting a corrupted name and rebuilding forever.
    unsupported = ordinary / "unsupported "
    unsupported.write_text("unsupported watch spelling\n")
    git("add", "unsupported ")
    refused = command(["cargo", "build", "--offline", "--locked", "-p", APP], check=False)
    (logs / "unsupported-watch-path.stderr.log").write_text(refused.stderr)
    rejection = {"name": "unsupported-watch-path", "checks": {
        "nonzero_exit": refused.returncode != 0,
        "exact_diagnostic": "Cargo cannot watch build-identity path without changing its spelling" in refused.stderr,
    }}
    observations.append(rejection)
    git("reset", "HEAD", "--", "unsupported ")
    unsupported.unlink()
    git("switch", "--detach", "HEAD")
    first = probe("detached-first", ordinary, marker="changed", ref_only=True)
    probe("detached-identical", ordinary, first, marker="changed")
    linked = output / "linked"
    git("worktree", "add", "--detach", str(linked), "HEAD")
    first = probe("linked-first", linked, marker="changed")
    probe("linked-identical", linked, first, marker="changed")
    git("commit", "--no-gpg-sign", "--allow-empty", "-m", "Advance linked HEAD", cwd=linked)
    probe("linked-ref-advance", linked, marker="changed", ref_only=True)
    archive = output / "archive"
    archive.mkdir()
    for filename in ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".gitignore"]:
        shutil.copyfile(ordinary / filename, archive / filename)
    shutil.copytree(ordinary / "crates", archive / "crates")
    first = probe("archive-first", archive, marker="changed")
    time.sleep(1.1)
    probe("archive-identical", archive, first, marker="changed")
    source_unchanged = all(digest(Path(path).read_bytes()) == expected for path, expected in sources.items())
    report = {"source_unchanged": source_unchanged, "sources": sources,
              "probe_sha256": digest(Path(__file__).read_bytes()), "observations": observations,
              "status": "passed" if source_unchanged and all(all(item["checks"].values()) for item in observations) else "failed",
              "scope": "Actual CLI build producer/identity constant + actual verum_error crash API/common dependencies, one representative transitive consumer. No actual verum_types/CLI/full stdlib/LLVM/AOT build.",
              "target": str(target)}
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"metadata probe: {report['status']}; {output / 'result.json'}", flush=True)
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", required=True, type=Path)
    parser.add_argument("--target-dir", required=True, type=Path)
    args = parser.parse_args()
    try:
        raise SystemExit(run_probe(args.evidence_dir.resolve(), args.target_dir.resolve()))
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        parser.exit(2, f"metadata probe error: {error}\n")
