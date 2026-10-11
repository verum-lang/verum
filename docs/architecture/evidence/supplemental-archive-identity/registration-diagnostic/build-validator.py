#!/usr/bin/env python3
"""Stage23: replay unchanged registry checks with explicit new build identities; no builds."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import time

REGISTRY_COMMIT = "6c055769bab8deca3787b55e782e7ca61a7ce2c4"
REGISTRY_TREE = "0a8e4d476bfbef99df285b3d5b34640494816cf3"
STAGE22_LANGUAGE_COMMIT = "e86f2abd77e041e2f29cdcbf30d9629435e64f7e"
STAGE22_CLI_SHA256 = "73d96b4c346d8d8eb759abf03c5c3316f3d349fb8e5601318d0d5da368979f10"
STAGE22_PLAN = Path("/private/tmp/verum-registry-stage22-boundary-inputs-v3/plan.json")
STAGE22_PLAN_SHA256 = "5015659b61b8cfe80df8a9d3c1b16d097acbbdab4682f0da26fec1dd615d9f17"
MODULES = (
    "src/protocol/authority.vr", "src/protocol/publication.vr",
    "src/protocol/publication_metadata.vr", "src/node/publication_auth.vr",
    "src/node/publication_prepare.vr",
)
MANIFEST = b'[cog]\nname = "verum-registry"\nversion = "0.1.0"\nedition = "2026"\n'
CASES = {
    "publication_prepare_consume": {"kind": "positive"},
    "publication_auth_private_store": {
        "kind": "privacy", "fields": ["credentials"],
        "owner": "verum_registry.node.publication_auth", "needle": "store.credentials",
    },
    "publication_auth_construct_store": {
        "kind": "privacy", "fields": ["principals", "credentials"],
        "owner": "verum_registry.node.publication_auth", "needle": "CredentialStore { principals:",
    },
    "publication_prepare_construct": {
        "kind": "privacy", "fields": ["attribution", "publication"],
        "owner": "verum_registry.node.publication_prepare", "needle": "PreparedPublication { attribution:",
    },
    "publication_prepare_private_field": {
        "kind": "privacy", "fields": ["publication"],
        "owner": "verum_registry.node.publication_prepare", "needle": "prepared.publication",
    },
    "publication_prepare_affine_reuse": {"kind": "affine"},
}
ARTIFACTS = ("runtime.vbca", "runtime.core_metadata", "runtime.symbol_graph",
             "runtime.vbca.checksum", "runtime.vbca.schema")
SCOPE = (
    "Ordinary argument-less project type checking with exact registry production "
    "modules. Negative acceptance requires only the specified diagnostic(s), "
    "declaring owner and fixture location. This does not execute the fixture or "
    "establish shared-copy isolation, HTTP, durable admission, download, or AOT."
)


def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def sha_bytes(value):
    return hashlib.sha256(value).hexdigest()


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args])


def require(condition, message):
    if not condition:
        raise ValueError(message)


def write_json(path, value):
    with path.open("x", encoding="utf-8") as destination:
        json.dump(value, destination, indent=2)
        destination.write("\n")


def snapshot(root):
    files = {}
    for path in sorted(root.rglob("*")):
        require(not path.is_symlink(), f"symlink is not an input: {path}")
        if path.is_file():
            files[str(path.relative_to(root))] = digest(path)
        else:
            require(path.is_dir(), f"non-regular input: {path}")
    return files


def registry_identity(root):
    require(git(root, "rev-parse", "HEAD").decode().strip() == REGISTRY_COMMIT,
            "registry HEAD differs from the frozen candidate")
    require(not git(root, "status", "--porcelain").strip(), "registry must be clean")
    tree = git(root, "rev-parse", "HEAD^{tree}").decode().strip()
    require(tree == REGISTRY_TREE, "registry tree differs from the frozen candidate")
    return tree


def fixture_oracle(name, entry):
    oracle = dict(CASES[name])
    if oracle["kind"] == "privacy":
        needle = oracle.pop("needle").encode()
        require(entry.count(needle) == 1, "ambiguous private access fixture")
        oracle["lines"] = [entry[:entry.index(needle)].count(b"\n") + 1]
        oracle["code"] = "E202"
    elif oracle["kind"] == "affine":
        needle = b"prepared.into_parts()"
        offsets = [match.start() for match in re.finditer(re.escape(needle), entry)]
        require(len(offsets) == 2, "affine fixture must retain both consuming uses")
        oracle.update(code="E310", variable="prepared", moved_at=offsets[0], used_at=offsets[1],
                      lines=[entry[:offset].count(b"\n") + 1 for offset in offsets])
    return oracle


def frozen_plan_inputs(plan):
    """Compare only input bytes/oracles, never inherit a stage22 build or result."""
    require(digest(STAGE22_PLAN) == STAGE22_PLAN_SHA256, "frozen stage22 input plan changed")
    baseline = json.loads(STAGE22_PLAN.read_text())
    for key in ("registry_commit", "registry_tree", "source_inputs"):
        require(plan.get(key) == baseline[key], f"frozen registry inputs differ: {key}")
    require(set(plan["cases"]) == set(CASES), "all six frozen cases required")
    for name in CASES:
        for key in ("fixture", "project_inputs", "oracle"):
            require(plan["cases"][name].get(key) == baseline["cases"][name][key],
                    f"frozen boundary input differs: {name}/{key}")
    return {"path": str(STAGE22_PLAN), "sha256": STAGE22_PLAN_SHA256,
            "role": "Input/oracle provenance only; no build identity or outcome reused."}


def prepare(args):
    source = args.registry.resolve(strict=True)
    tree = registry_identity(source)
    destination = args.inputs_dir.resolve()
    destination.mkdir(exist_ok=False)
    roster = (*MODULES, "tools/run_publication_envelope.py",
              *(f"tests/language/{name}.vr" for name in CASES))
    data = {name: (source / name).read_bytes() for name in roster}
    for name, value in data.items():
        require(value == git(source, "show", f"{REGISTRY_COMMIT}:{name}"),
                f"source differs from committed bytes: {name}")
        target = destination / "source" / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(value)
    plan = {
        "registry_commit": REGISTRY_COMMIT, "registry_tree": tree,
        "registry_path": str(source), "scope": SCOPE,
        "source_inputs": {name: sha_bytes(value) for name, value in data.items()},
        "runner_sha256": digest(Path(__file__)), "cases": {},
        "build_identity": "Not bound yet: run requires a successful fresh manifest and explicit manifest/CLI/source pins.",
        "entry_transformation": "Replace only the one complete fixture module-header line; preserve all other fixture and production bytes.",
    }
    for name in CASES:
        fixture = f"tests/language/{name}.vr"
        header = f"module verum_registry.tests.{name};\n".encode()
        require(data[fixture].count(header) == 1, f"unexpected fixture header: {fixture}")
        entry = data[fixture].replace(header, b"module verum_registry.main;\n", 1)
        inputs = {**{module: data[module] for module in MODULES},
                  "src/main.vr": entry, "verum.toml": MANIFEST}
        project = destination / "projects" / name
        for relative, value in inputs.items():
            path = project / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(value)
        case = {"fixture": fixture, "project_inputs": snapshot(project),
                "oracle": fixture_oracle(name, entry)}
        if args.previous_evidence:
            previous = args.previous_evidence / name / "result.json"
            old = json.loads(previous.read_text())
            case["previous_receipt"] = {"path": str(previous), "sha256": digest(previous),
                "registry_commit": old["registry_commit"],
                "source_comparison": {
                    relative: {"previous": old["source_inputs"].get(relative),
                               "current": plan["source_inputs"][relative],
                               "unchanged": old["source_inputs"].get(relative) == plan["source_inputs"][relative]}
                    for relative in (*MODULES, fixture)}}
        plan["cases"][name] = case
    require(registry_identity(source) == tree, "registry changed during preparation")
    require(all(digest(source / name) == value for name, value in plan["source_inputs"].items()),
            "source bytes changed during preparation")
    plan["frozen_input_baseline"] = frozen_plan_inputs(plan)
    write_json(destination / "plan.json", plan)
    print(json.dumps({"state": "prepared_not_executed", "plan": str(destination / "plan.json"),
                      "plan_sha256": digest(destination / "plan.json"), "cases": list(CASES)}, indent=2))


def validate_build(path, manifest_sha, cli_sha, language_commit, expected_schema, verum):
    require(re.fullmatch(r"[0-9a-f]{64}", manifest_sha) is not None, "full manifest SHA-256 required")
    require(re.fullmatch(r"[0-9a-f]{64}", cli_sha) is not None, "full CLI SHA-256 required")
    require(re.fullmatch(r"[0-9a-f]{40}", language_commit) is not None, "full language commit required")
    require(language_commit != STAGE22_LANGUAGE_COMMIT and cli_sha != STAGE22_CLI_SHA256,
            "stage22 language/CLI identity cannot be relabeled as stage23")
    require(isinstance(expected_schema, str) and expected_schema.strip() == expected_schema
            and bool(expected_schema) and "\n" not in expected_schema, "explicit archive schema required")
    require(digest(path) == manifest_sha, "build manifest hash mismatch")
    build = json.loads(path.read_text())
    require(build.get("status") == "passed" and build.get("exit_code") == 0
            and build.get("source_unchanged") is True
            and build.get("source_paths_unchanged") is True
            and build.get("automatic_precompile") is True
            and build.get("timed_out") is False and build.get("interrupted") is False,
            "build is not a successful unchanged automatic bake")
    require(build.get("engine_commit") == language_commit, "language source pin mismatch")
    require(re.fullmatch(r"[0-9a-f]{40}", language_commit) is not None, "full language commit required")
    cli = Path(build["cli"]["path"]).resolve(strict=True)
    require(cli == Path(verum).resolve(strict=True), "explicit CLI path differs from build manifest")
    require(cli.is_file() and os.access(cli, os.X_OK), "CLI is not executable")
    require(build["cli"]["sha256"] == cli_sha == digest(cli), "CLI hash mismatch")
    require(cli.stat().st_size == build["cli"]["bytes"], "CLI size mismatch")
    require(build.get("schema") == expected_schema
            and build.get("expected_schema") == expected_schema, "fresh archive schema mismatch")
    require(all(name in build.get("artifacts", {}) for name in ARTIFACTS), "five archive identities required")
    artifact_root = Path(build["artifact_snapshot"]).resolve(strict=True)
    for name in ARTIFACTS:
        identity = build["artifacts"][name]
        require(digest(artifact_root / name) == identity["sha256"], f"archive hash mismatch: {name}")
        require((artifact_root / name).stat().st_size == identity["bytes"], f"archive size mismatch: {name}")
        require(build.get("artifacts_after", {}).get(name) == identity,
                f"retained archive differs from the completed build: {name}")
    require((artifact_root / "runtime.vbca.schema").read_text().strip() == expected_schema,
            "retained schema file mismatch")
    fingerprint = (artifact_root / "runtime.vbca.checksum").read_text().strip()
    require(re.fullmatch(r"[0-9a-f]{64}", fingerprint) is not None
            and fingerprint == build.get("source_fingerprint_blake3"),
            "retained source checksum mismatch")
    return build, cli


def classify(oracle, project, stdout, stderr, returncode, timed_out=False, unchanged=True):
    text = stdout.decode("utf-8", "replace") + "\n" + stderr.decode("utf-8", "replace")
    text = re.sub(r"\x1b\[[0-9;]*m", "", text)
    headers = list(re.finditer(r"^error(?:<(?P<angle>E\d+)>|\[(?P<bracket>E\d+)\]): (?P<message>[^\n]+)$", text, re.M))
    diagnostics = []
    for index, header in enumerate(headers):
        block = text[header.end():headers[index + 1].start() if index + 1 < len(headers) else len(text)]
        locations = re.findall(r"^\s*--> (.+):(\d+):(\d+)\s*$", block, re.M)
        diagnostics.append({"code": header.group("angle") or header.group("bracket"),
                            "message": header.group("message"), "locations": locations,
                            "moved_at": re.findall(r"^\s*moved at: (\d+)\s*$", block, re.M),
                            "used_at": re.findall(r"^\s*used at: (\d+)\s*$", block, re.M)})
    errors = [line for line in text.splitlines() if re.match(r"^error\b", line, re.I)]
    summaries = [int(match.group(1)) for line in errors
                 if (match := re.fullmatch(r"error: compilation failed with (\d+) errors?", line))]
    unknown_errors = [line for line in errors if not re.fullmatch(
        r"error(?:<E\d+>|\[E\d+\]): [^\n]+|error: compilation failed with \d+ errors?", line)]
    crash = bool(re.search(r"panicked at|internal compiler error|runtime error:|segmentation fault|stack overflow", text, re.I))
    expected_file = str(project / "src/main.vr")
    exact = []
    for diagnostic in diagnostics:
        located = bool(diagnostic["locations"]) and all(
            path == expected_file and int(line) in oracle.get("lines", [])
            for path, line, _ in diagnostic["locations"])
        if oracle["kind"] == "privacy":
            messages = {f"visibility error: '{field}' is Private in module '{oracle['owner']}'"
                        for field in oracle["fields"]}
            exact.append(diagnostic["code"] == "E202" and diagnostic["message"] in messages and located)
        elif oracle["kind"] == "affine":
            exact.append(diagnostic["code"] == "E310" and
                         diagnostic["message"] == "value `prepared` used after move" and located and
                         diagnostic["moved_at"] == [str(oracle["moved_at"])] and
                         diagnostic["used_at"] == [str(oracle["used_at"])])
    completed = "Finished checking verum-registry in " in text and re.search(r"\b6 files checked,", text) is not None
    if timed_out:
        verdict = "inconclusive_timeout"
    elif not unchanged:
        verdict = "inconclusive_changed_inputs"
    elif returncode is None or returncode < 0 or crash:
        verdict = "inconclusive_process_failure"
    elif oracle["kind"] == "positive":
        verdict = "positive_check_passed" if returncode == 0 and completed and not errors else "positive_check_failed"
    elif returncode == 0:
        verdict = "unexpected_acceptance"
    elif diagnostics and all(exact) and not unknown_errors and summaries and all(n == len(diagnostics) for n in summaries):
        verdict = "specific_refusal_passed"
    else:
        verdict = "inconclusive_unexpected_diagnostics"
    return {"verdict": verdict, "accepted": verdict in ("positive_check_passed", "specific_refusal_passed"),
            "diagnostics": diagnostics, "unknown_error_lines": unknown_errors,
            "compilation_failure_counts": summaries, "complete_check_marker": completed}


def stop(process):
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        return process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        return process.wait(timeout=10)


def run(args):
    require(math.isfinite(args.timeout) and 0 < args.timeout <= 600, "deadline must be positive and at most 600 seconds")
    plan_path = args.plan.resolve(strict=True)
    require(digest(plan_path) == args.expected_plan_sha256, "input plan hash mismatch")
    plan = json.loads(plan_path.read_text())
    require(plan["registry_commit"] == REGISTRY_COMMIT, "registry plan pin mismatch")
    frozen_inputs = frozen_plan_inputs(plan)
    require(plan["runner_sha256"] == digest(Path(__file__)), "runner changed after preparation")
    source = Path(plan["registry_path"])
    require(registry_identity(source) == plan["registry_tree"], "registry tree changed")
    require(all(digest(source / name) == value for name, value in plan["source_inputs"].items()), "registry source changed")
    require(snapshot(plan_path.parent / "source") == plan["source_inputs"], "retained source snapshot changed")
    build_path = args.build_manifest.resolve(strict=True)
    build, cli = validate_build(build_path, args.expected_build_manifest_sha256,
                                args.expected_cli_sha256, args.language_commit, args.expected_schema, args.verum)
    evidence = args.evidence_dir.resolve()
    evidence.mkdir(exist_ok=False)
    shutil.copyfile(plan_path, evidence / "input-plan.json")
    shutil.copyfile(build_path, evidence / "build-manifest.json")
    shutil.copyfile(Path(__file__), evidence / "runner.py")
    removed = sorted(name for name in os.environ if name.startswith("VERUM_"))
    overrides = {"NO_COLOR": "1", "VERUM_NO_OBJECT_CACHE": "1", "TMPDIR": "/private/tmp"}
    environment = {name: value for name, value in os.environ.items() if not name.startswith("VERUM_")}
    environment.update(overrides)
    receipts = []
    for name in (CASES if args.case == "all" else (args.case,)):
        case = plan["cases"][name]
        retained = plan_path.parent / "projects" / name
        require(snapshot(retained) == case["project_inputs"], f"prepared project changed: {name}")
        destination = evidence / name
        destination.mkdir()
        project = destination / "project"
        shutil.copytree(retained, project)
        require(snapshot(project) == case["project_inputs"], "project copy mismatch")
        require(case["oracle"] == fixture_oracle(name, (project / "src/main.vr").read_bytes()), "oracle changed")
        validate_build(build_path, args.expected_build_manifest_sha256, args.expected_cli_sha256, args.language_commit, args.expected_schema, args.verum)
        command = [str(cli), "check"]
        record = {"stage": "stage23", "frozen_input_baseline": frozen_inputs, "task": "T1636", "related_tasks": ["T1713", "T1722"], "case": name,
                  "scope": SCOPE, "command": command, "working_directory": str(project),
                  "environment_overrides": overrides, "removed_environment_keys": removed,
                  "environment_policy": "Remove inherited VERUM_* overrides before adding the recorded ordinary-check overrides; no trace environment is retained.",
                  "language_commit": args.language_commit, "cli": build["cli"],
                  "build_manifest_sha256": args.expected_build_manifest_sha256,
                  "schema": build["schema"], "artifact_identities": build["artifacts"],
                  "engine_tree": build.get("engine_tree"),
                  "source_fingerprint_blake3": build["source_fingerprint_blake3"],
                  "producer_before": build.get("producer_before"), "producer_after": build.get("producer_after"),
                  "input_plan_sha256": args.expected_plan_sha256,
                  "registry_commit": REGISTRY_COMMIT, "registry_tree": plan["registry_tree"],
                  "source_inputs": plan["source_inputs"], "project_inputs": case["project_inputs"],
                  "oracle": case["oracle"], "deadline_seconds": args.timeout,
                  "entry_transformation": plan["entry_transformation"]}
        write_json(destination / "started.json", record)
        print(json.dumps({"case": name, "state": "started"}), flush=True)
        started = time.monotonic()
        timed_out = False
        error = None
        code = None
        with (destination / "stdout.log").open("xb") as stdout, (destination / "stderr.log").open("xb") as stderr:
            try:
                process = subprocess.Popen(command, cwd=project, env=environment, stdout=stdout,
                                           stderr=stderr, start_new_session=True)
                try:
                    code = process.wait(timeout=args.timeout)
                except subprocess.TimeoutExpired:
                    timed_out = True
                    code = stop(process)
                except KeyboardInterrupt:
                    code = stop(process)
                    error = "interrupted"
            except (OSError, subprocess.TimeoutExpired) as problem:
                error = str(problem)
        probes = {
            "project_unchanged": lambda: snapshot(project) == case["project_inputs"],
            "retained_project_unchanged": lambda: snapshot(retained) == case["project_inputs"],
            "source_unchanged": lambda: all(digest(source / item) == value for item, value in plan["source_inputs"].items()),
            "retained_source_unchanged": lambda: snapshot(plan_path.parent / "source") == plan["source_inputs"],
            "registry_unchanged": lambda: registry_identity(source) == plan["registry_tree"],
            "plan_unchanged": lambda: digest(plan_path) == args.expected_plan_sha256,
            "runner_unchanged": lambda: digest(Path(__file__)) == plan["runner_sha256"],
            "retained_runner_unchanged": lambda: digest(evidence / "runner.py") == plan["runner_sha256"],
            "retained_build_manifest_unchanged": lambda: digest(evidence / "build-manifest.json") == args.expected_build_manifest_sha256,
            "frozen_baseline_unchanged": lambda: frozen_plan_inputs(plan) == frozen_inputs,
        }
        checks = {}
        identity_errors = {}
        for key, probe in probes.items():
            try:
                checks[key] = bool(probe())
            except (OSError, ValueError, subprocess.CalledProcessError) as problem:
                checks[key] = False
                identity_errors[key] = str(problem)
        try:
            validate_build(build_path, args.expected_build_manifest_sha256, args.expected_cli_sha256, args.language_commit, args.expected_schema, args.verum)
            checks["build_and_artifacts_unchanged"] = True
        except (OSError, ValueError, KeyError) as problem:
            checks["build_and_artifacts_unchanged"] = False
            error = str(problem)
        record.update(returncode=code, elapsed_seconds=round(time.monotonic() - started, 3),
                      timed_out=timed_out, runner_error=error, identity_checks=checks, identity_errors=identity_errors,
                      logs={item: digest(destination / item) for item in ("stdout.log", "stderr.log")})
        record.update(classify(case["oracle"], project, (destination / "stdout.log").read_bytes(),
                               (destination / "stderr.log").read_bytes(), code, timed_out,
                               all(checks.values()) and error is None))
        write_json(destination / "result.json", record)
        receipts.append({key: record[key] for key in ("case", "returncode", "elapsed_seconds", "verdict", "accepted")})
        print(json.dumps(receipts[-1]), flush=True)
        if not all(checks.values()) or error is not None:
            break
    expected_count = len(CASES) if args.case == "all" else 1
    passed = len(receipts) == expected_count and all(item["accepted"] for item in receipts)
    write_json(evidence / "summary.json", {"stage": "stage23", "cases": receipts,
        "expected_case_count": expected_count, "all_selected_completed": len(receipts) == expected_count,
        "all_selected_accepted": passed, "scope": SCOPE})
    return 0 if passed else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    prep = sub.add_parser("prepare")
    prep.add_argument("--registry", type=Path, default=Path("/private/tmp/verum-registry-t1636-auth"))
    prep.add_argument("--inputs-dir", type=Path, required=True)
    prep.add_argument("--previous-evidence", type=Path)
    replay = sub.add_parser("run")
    replay.add_argument("--plan", type=Path, required=True)
    replay.add_argument("--expected-plan-sha256", required=True)
    replay.add_argument("--verum", type=Path, required=True)
    replay.add_argument("--expected-schema", required=True)
    replay.add_argument("--build-manifest", type=Path, required=True)
    replay.add_argument("--expected-build-manifest-sha256", required=True)
    replay.add_argument("--expected-cli-sha256", required=True)
    replay.add_argument("--language-commit", required=True)
    replay.add_argument("--evidence-dir", type=Path, required=True)
    replay.add_argument("--case", choices=["all", *CASES], required=True)
    replay.add_argument("--timeout", type=float, default=600)
    args = parser.parse_args()
    try:
        if args.action == "prepare":
            prepare(args)
            return 0
        return run(args)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(2, f"PREFLIGHT/IDENTITY REFUSED: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
