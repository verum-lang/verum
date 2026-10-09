#!/usr/bin/env python3
"""Measure the actual verum_error build script in a dependency-free Cargo fixture.

This is an explicit compiler-build probe, not a workspace test. It never uses a
workspace Cargo target, changes production source, or downloads dependencies.
Exit 1 records violated metadata/cache requirements; exit 2 is a harness error.
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
COMMAND = ["cargo", "test", "--offline", "--locked", "-p",
           "verum_build_metadata_fixture", "--", "--nocapture"]
LIBRARY = '''#[test]
fn captured_metadata() {
    println!("SOURCE_MARKER=initial");
    println!("CAPTURED_SHA={}", env!("VERUM_BUILD_GIT_SHA"));
    println!("CAPTURED_DIRTY={}", env!("VERUM_BUILD_GIT_DIRTY"));
    println!("CAPTURED_TIMESTAMP={}", env!("VERUM_BUILD_TIMESTAMP"));
}
'''


def digest(data):
    return hashlib.sha256(data).hexdigest()


def run_probe(output):
    output.mkdir()  # Refuse to overwrite earlier evidence.
    source = ROOT / "crates/verum_error/build.rs"
    script = source.read_bytes()
    ordinary = output / "ordinary"
    package = ordinary / "crates/build_metadata_fixture"
    (package / "src").mkdir(parents=True)
    (ordinary / "Cargo.toml").write_text(
        '[workspace]\nmembers=["crates/build_metadata_fixture"]\nresolver="3"\n')
    (package / "Cargo.toml").write_text(
        '[package]\nname="verum_build_metadata_fixture"\nversion="0.0.0"\nedition="2024"\n')
    (package / "build.rs").write_bytes(script)
    (package / "src/lib.rs").write_text(LIBRARY)
    (ordinary / ".gitignore").write_text("/target/\n")
    logs = output / "logs"
    logs.mkdir()
    target = output / "target"
    overrides = {
        "CARGO_TARGET_DIR": str(target), "CARGO_INCREMENTAL": "0",
        "CARGO_BUILD_JOBS": "2", "CARGO_PROFILE_DEV_DEBUG": "0",
        "CARGO_LOG": "cargo::core::compiler::fingerprint=trace",
        "VERUM_NO_AUTO_PRECOMPILE": "1", "TMPDIR": str(output),
        "CARGO_TERM_COLOR": "never",
    }
    environment = {**os.environ, **overrides}

    def command(args, cwd=ordinary, check=True, extra_env=None):
        result = subprocess.run(args, cwd=cwd, env={**environment, **(extra_env or {})},
                                capture_output=True, text=True, timeout=60)
        if check and result.returncode:
            raise RuntimeError(f"{args!r} failed: {result.stderr}")
        return result

    def git(*args, cwd=ordinary, check=True):
        return command(["git", *args], cwd, check)

    git("init")
    git("config", "user.name", "Verum Build Fixture")
    git("config", "user.email", "fixture@example.invalid")
    git("config", "commit.gpgsign", "false")
    git("config", "gc.auto", "0")
    hooks = output / "empty-hooks"
    hooks.mkdir()
    git("config", "core.hooksPath", str(hooks))
    command(["cargo", "generate-lockfile", "--offline"])
    git("add", "Cargo.toml", "Cargo.lock", ".gitignore", "crates")
    git("commit", "--no-gpg-sign", "-m", "Build metadata fixture")
    linked = output / "linked"
    git("worktree", "add", "--detach", str(linked), "HEAD")
    checks = []
    observations = []

    def probe(name, cwd, previous=None, marker="initial"):
        # Keep layouts independent. Switching cwd with one Cargo target can
        # otherwise reuse the prior layout's metadata before its first probe.
        layout_target = target / cwd.name
        revision = git("rev-parse", "--short=12", "HEAD", cwd=cwd, check=False)
        status = git("status", "--porcelain", cwd=cwd, check=False)
        expected_sha = revision.stdout.strip() if revision.returncode == 0 else "unknown"
        expected_dirty = ("dirty" if status.stdout.strip() else "clean") if status.returncode == 0 else "unknown"
        fixture_script = cwd / "crates/build_metadata_fixture/build.rs"
        input_hashes = {name: digest((cwd / name).read_bytes()) for name in [
            "Cargo.toml", "Cargo.lock", "crates/build_metadata_fixture/Cargo.toml",
            "crates/build_metadata_fixture/build.rs", "crates/build_metadata_fixture/src/lib.rs",
        ]}
        start = time.monotonic()
        result = command(COMMAND, cwd, check=False,
                         extra_env={"CARGO_TARGET_DIR": str(layout_target)})
        stdout_name = f"{name}.stdout.log"
        stderr_name = f"{name}.stderr.log"
        (logs / stdout_name).write_text(result.stdout)
        (logs / stderr_name).write_text(result.stderr)
        if result.returncode:
            raise RuntimeError(f"Cargo fixture failed; see {stderr_name}")
        captured = dict(line.split("=", 1) for line in result.stdout.splitlines()
                        if line.startswith(("CAPTURED_", "SOURCE_MARKER=")))
        executables = [path for path in (layout_target / "debug/deps").glob("verum_build_metadata_fixture-*")
                       if path.is_file() and path.suffix == "" and os.access(path, os.X_OK)]
        identity = {str(path.relative_to(layout_target)): {
            "sha256": digest(path.read_bytes()), "mtime_ns": path.stat().st_mtime_ns,
        } for path in executables}
        verdicts = {
            "git_sha": captured.get("CAPTURED_SHA") == expected_sha,
            "git_dirty": captured.get("CAPTURED_DIRTY") == expected_dirty,
            "compiled_source_marker": captured.get("SOURCE_MARKER") == marker,
            "test_executable_identified": bool(identity),
            "actual_build_script_unchanged": fixture_script.read_bytes() == script,
            "fixture_inputs_unchanged_during_invocation": all(
                digest((cwd / name).read_bytes()) == checksum
                for name, checksum in input_hashes.items()),
        }
        if previous:
            verdicts["identical_fixture_inputs"] = input_hashes == previous["fixture_inputs_sha256"]
            verdicts["unchanged_executable_reused"] = identity == previous["executables"]
            verdicts["unchanged_timestamp"] = captured.get("CAPTURED_TIMESTAMP") == previous["captured"].get("CAPTURED_TIMESTAMP")
        record = {
            "name": name, "working_directory": str(cwd), "command": COMMAND,
            "cargo_target_dir": str(layout_target),
            "elapsed_seconds": round(time.monotonic() - start, 3),
            "returncode": result.returncode,
            "expected_git_sha": expected_sha, "expected_git_dirty": expected_dirty,
            "captured": captured, "executables": identity, "checks": verdicts,
            "fixture_inputs_sha256": input_hashes,
            "git_marker_kind": ("file" if (cwd / ".git").is_file() else
                                "directory" if (cwd / ".git").is_dir() else "absent"),
            "legacy_watch_path_exists": (fixture_script.parent / "../../.git/HEAD").exists(),
            "logs": {stdout_name: digest(result.stdout.encode()),
                     stderr_name: digest(result.stderr.encode())},
        }
        observations.append(record)
        checks.extend(verdicts.values())
        return record

    for name, cwd in [("linked", linked), ("ordinary", ordinary)]:
        first = probe(f"{name}-first", cwd)
        time.sleep(1.1)  # Cross the timestamp's one-second resolution.
        probe(f"{name}-identical", cwd, first)

    library = package / "src/lib.rs"
    library.write_text(LIBRARY.replace("SOURCE_MARKER=initial", "SOURCE_MARKER=changed"))
    probe("ordinary-source-dirty", ordinary, marker="changed")
    git("add", "crates/build_metadata_fixture/src/lib.rs")
    git("commit", "--no-gpg-sign", "-m", "Change compiled source")
    probe("ordinary-committed-source", ordinary, marker="changed")
    git("commit", "--no-gpg-sign", "--allow-empty", "-m", "Advance symbolic ref")
    probe("ordinary-ref-advance", ordinary, marker="changed")
    git("switch", "--detach", "HEAD")
    first = probe("detached-first", ordinary, marker="changed")
    time.sleep(1.1)
    probe("detached-identical", ordinary, first, marker="changed")

    nogit = output / "without-git"
    nogit.mkdir()
    for name in ["Cargo.toml", "Cargo.lock", ".gitignore"]:
        shutil.copyfile(ordinary / name, nogit / name)
    shutil.copytree(ordinary / "crates", nogit / "crates")
    first = probe("no-git-first", nogit, marker="changed")
    time.sleep(1.1)
    probe("no-git-identical", nogit, first, marker="changed")
    source_unchanged = source.read_bytes() == script
    report = {
        "task": "T1683", "source_build_script_sha256": digest(script),
        "harness_sha256": digest(Path(__file__).read_bytes()),
        "source_revision": command(["git", "rev-parse", "HEAD"], ROOT).stdout.strip(),
        "source_unchanged": source_unchanged, "environment_overrides": overrides,
        "cargo_version": command(["cargo", "--version"]).stdout.strip(),
        "rustc_version_on_path": command(["rustc", "--version"]).stdout.strip(),
        "observations": observations,
        "status": "passed" if source_unchanged and all(checks) else "failed",
        "scope": "Dependency-free Cargo fixture containing the actual build script; no workspace compiler, LLVM, Z3, bake or AOT acceptance.",
    }
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"build metadata probe: {report['status']}; {output / 'result.json'}")
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-dir", required=True, type=Path,
                        help="new directory beneath an existing parent; includes the small target")
    arguments = parser.parse_args()
    try:
        raise SystemExit(run_probe(arguments.evidence_dir.resolve()))
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        parser.exit(2, f"build metadata probe error: {error}\n")
