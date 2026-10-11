#!/usr/bin/env python3
"""Prepare locked SemVer oracles or compare retained ordinary Verum stdout.

Never invokes Verum, Cargo, or a bake. Runtime execution needs an independently
pinned CLI; incomplete/malformed output is a runner failure, not a semantic pass.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import signal
import subprocess
import tarfile
import tomllib


ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / "core-tests/base/semver_constraint/publication_cases.json"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_cases(path):
    corpus = json.loads(path.read_text())
    cases = corpus["cases"]
    require(cases, "empty case corpus")
    require(len({c["id"] for c in cases}) == len(cases), "duplicate case id")
    for case in cases:
        require(re.fullmatch(r"[a-z][a-z0-9_]*", case["id"]), "invalid case id")
        require(case["track"] in ("publication", "extension"), "unknown contract track")
        require(type(case["accepts"]) is bool, "acceptance must be explicit")
        require(isinstance(case["requirement"], str), "a missing requirement is not a wildcard")
        require(case["accepts"] or not case["candidates"], "rejected case has matching oracle")
        for candidate in case["candidates"]:
            require(type(candidate["matches"]) is bool, "match oracle must be explicit")
    return corpus


def parse_output(output, cases):
    count = len(cases)
    require(count > 0, "cannot accept zero coverage")
    lines = output.splitlines()
    require(lines and lines[0] == f"semver-contract|begin|{count}", "missing exact start marker")
    require(lines[-1] == f"semver-contract|end|{count}", "missing exact completion marker")
    expected = {case["id"]: case for case in cases}
    records = {}
    for line in lines[1:-1]:
        parts = line.split("|")
        require(len(parts) >= 2 and parts[1] in expected, f"unknown result row: {line!r}")
        kind, name = parts[:2]
        record = records.setdefault(name, {"accepted": None, "matches": {}})
        if kind == "parse":
            require(len(parts) == 3 and parts[2] in ("0", "1"), f"malformed parse row: {line!r}")
            require(record["accepted"] is None, f"duplicate parse result: {name}")
            record["accepted"] = parts[2] == "1"
        elif kind == "match":
            require(len(parts) == 4 and parts[2].isdigit() and parts[3] in ("0", "1"),
                    f"malformed match row: {line!r}")
            index = int(parts[2])
            require(record["accepted"] is True, f"match before accepted parse: {name}")
            require(index < len(expected[name]["candidates"]), f"out-of-range candidate: {name}")
            require(index not in record["matches"], f"duplicate candidate: {name}/{index}")
            record["matches"][index] = parts[3] == "1"
        else:
            raise ValueError(f"non-semantic or unexpected result row: {line!r}")
    require(set(records) == set(expected), "missing case results")
    for name, record in records.items():
        require(record["accepted"] is not None, f"missing parse result: {name}")
        required = len(expected[name]["candidates"]) if record["accepted"] else 0
        require(len(record["matches"]) == required, f"incomplete matching set: {name}")
    return records


def differences(records, cases, reference=False):
    result = []
    for case in cases:
        name = case["id"]
        actual = records[name]
        accepted = case["accepts"] and not (reference and case["track"] == "extension")
        if actual["accepted"] != accepted:
            result.append(dict(case=name, axis="acceptance", expected=accepted, actual=actual["accepted"]))
        if accepted and actual["accepted"]:
            for index, candidate in enumerate(case["candidates"]):
                matched = actual["matches"][index]
                if matched != candidate["matches"]:
                    result.append(dict(case=name, axis="matching", version=candidate["version"],
                                       expected=candidate["matches"], actual=matched))
    return result


def literal(value):
    # Both languages use these JSON escapes; keep Unicode literal (no JSON-only \uXXXX).
    return json.dumps(value, ensure_ascii=False)


def rust_source(cases):
    calls = []
    for case in cases:
        versions = ", ".join(literal(c["version"]) for c in case["candidates"])
        calls.append(f'    observe({literal(case["id"])}, {literal(case["requirement"])}, &[{versions}]);')
    return '''use semver::{Version, VersionReq};
fn observe(id: &str, requirement: &str, versions: &[&str]) {
    match VersionReq::parse(requirement) {
        Ok(constraint) => {
            println!("parse|{id}|1");
            for (index, spelling) in versions.iter().enumerate() {
                let version = Version::parse(spelling).expect("invalid candidate fixture");
                println!("match|{id}|{index}|{}", u8::from(constraint.matches(&version)));
            }
        }
        Err(error) => {
            println!("parse|{id}|0");
            eprintln!("rejection|{id}|{error}");
        }
    }
}
fn main() {
''' + f'    println!("semver-contract|begin|{len(cases)}");\n' + "\n".join(calls) + f'\n    println!("semver-contract|end|{len(cases)}");\n}}\n'


def verum_source(cases, entry_point="parse"):
    require(entry_point in ("parse", "parse_cargo", "parse_extended"), "unknown parser entry point")
    if not any(case["candidates"] for case in cases):
        calls = [f'    observe(&{literal(c["id"])}, &{literal(c["requirement"])});' for c in cases]
        return ("// Requirement parsing only; no version-parser or matcher dependency.\n"
                f"mount core.base.semver_constraint.{{{entry_point} as parse_requirement}};\n\n"
                "fn observe(id: &Text, requirement: &Text) {\n"
                "    match parse_requirement(requirement) {\n"
                '        Result.Ok(_) => print(f"parse|{id}|1"),\n'
                '        Result.Err(_) => print(f"parse|{id}|0"),\n'
                "    }\n}\n\nfn main() {\n"
                f'    print("semver-contract|begin|{len(cases)}");\n'
                + "\n".join(calls)
                + f'\n    print("semver-contract|end|{len(cases)}");\n}}\n')
    calls = []
    for case in cases:
        versions = ", ".join(literal(c["version"]) for c in case["candidates"])
        calls.append(f'    observe(&{literal(case["id"])}, &{literal(case["requirement"])}, [{versions}]);')
    source = '''// Public parser/matcher observations; expected results live in the comparison corpus.
mount core.base.semver.SemVer;
mount core.base.semver_constraint.{parse as parse_requirement, matches_default};

fn observe(id: &Text, requirement: &Text, versions: List<Text>) {
    match parse_requirement(requirement) {
        Result.Ok(constraint) => {
            print(f"parse|{id}|1");
            let mut index: Int = 0;
            while index < versions.len() {
                match SemVer.parse(&versions[index]) {
                    Result.Ok(version) => {
                        if matches_default(&constraint, &version) {
                            print(f"match|{id}|{index}|1");
                        } else {
                            print(f"match|{id}|{index}|0");
                        }
                    },
                    Result.Err(_) => print(f"candidate-error|{id}|{index}"),
                }
                index = index + 1;
            }
        },
        Result.Err(_) => print(f"parse|{id}|0"),
    }
}

fn main() {
''' + f'    print("semver-contract|begin|{len(cases)}");\n' + "\n".join(calls) + f'\n    print("semver-contract|end|{len(cases)}");\n}}\n'
    return source.replace("parse as parse_requirement", f"{entry_point} as parse_requirement")


def run_recorded(command, cwd, output, name, timeout=60):
    with (output / f"{name}.stdout").open("wb") as stdout, (output / f"{name}.stderr").open("wb") as stderr:
        child = subprocess.Popen(command, cwd=cwd, stdout=stdout, stderr=stderr, start_new_session=True)
        try:
            code = child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            import os
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()
            raise ValueError(f"{name} exceeded {timeout}s; output retained")
    require(code == 0, f"{name} exited {code}; output retained")
    return dict(command=command, exit_code=code,
                stdout_sha256=sha(output / f"{name}.stdout"), stderr_sha256=sha(output / f"{name}.stderr"))


def prepare(args):
    corpus = read_cases(args.corpus)
    locked = [p for p in tomllib.loads((ROOT / "Cargo.lock").read_text())["package"] if p["name"] == "semver"]
    require(len(locked) == 1, "expected exactly one locked semver package")
    lock = locked[0]
    require(lock["version"] == corpus["reference"]["version"], "reference version differs from Cargo.lock")
    require(lock["checksum"] == corpus["reference"]["cargo_checksum"], "reference checksum differs from Cargo.lock")
    require(sha(args.semver_archive) == lock["checksum"], "cached semver archive differs from Cargo.lock")
    sources = {}
    with tarfile.open(args.semver_archive) as archive:
        for path in sorted((args.semver_source / "src").rglob("*.rs")):
            relative = path.relative_to(args.semver_source).as_posix()
            stored = archive.extractfile(f'semver-{lock["version"]}/{relative}')
            require(stored is not None and stored.read() == path.read_bytes(), f"cached source differs: {relative}")
            sources[relative] = sha(path)
    require("src/lib.rs" in sources and len(sources) >= 8, "incomplete semver source inventory")
    args.output.mkdir(parents=True, exist_ok=False)
    output = args.output.resolve()
    source = output / "reference.rs"
    source.write_text(rust_source(corpus["cases"]))
    rlib, executable = output / "libsemver.rlib", output / "semver-reference"
    commands = []
    commands.append(run_recorded([str(args.rustc), "--version", "--verbose"], output, output, "rustc-identity"))
    commands.append(run_recorded([str(args.rustc), "--edition=2021", "--crate-name", "semver", "--crate-type", "rlib",
                                 "--cfg", 'feature="std"', str(args.semver_source / "src/lib.rs"), "-o", str(rlib)], output, output, "reference-library"))
    commands.append(run_recorded([str(args.rustc), "--edition=2021", str(source), "--extern", f"semver={rlib}", "-o", str(executable)], output, output, "reference-build"))
    commands.append(run_recorded([str(executable)], output, output, "reference-run", 10))
    observations = parse_output((output / "reference-run.stdout").read_text(), corpus["cases"])
    delta = differences(observations, corpus["cases"], reference=True)
    fixtures = {}
    tracks = {
        "publication-acceptance": [dict(c, candidates=[]) for c in corpus["cases"] if c["track"] == "publication"],
        "publication-matching": [c for c in corpus["cases"] if c["track"] == "publication" and c["accepts"]],
        "declared-extensions": [c for c in corpus["cases"] if c["track"] == "extension"],
    }
    if args.api == "explicit":
        tracks["cargo-extension-refusal"] = [
            dict(c, track="publication", accepts=False, candidates=[])
            for c in corpus["cases"] if c["track"] == "extension"]
        tracks["compatibility-extensions"] = tracks["declared-extensions"]
    for name, cases in tracks.items():
        if not cases:
            continue  # A missing track is not a successful zero-coverage fixture.
        entry_point = "parse"
        if args.api == "explicit" and name != "compatibility-extensions":
            entry_point = "parse_extended" if name == "declared-extensions" else "parse_cargo"
        path = output / f"{name}.vr"
        path.write_text(verum_source(cases, entry_point))
        fixtures[name] = dict(path=str(path), sha256=sha(path), cases=cases, entry_point=entry_point)
    plan = dict(schema=2, api=args.api, reference=lock, reference_source_hashes=sources,
                reference_archive=str(args.semver_archive), reference_source=str(args.semver_source),
                corpus_path=str(args.corpus), corpus_sha256=sha(args.corpus),
                cargo_lock_sha256=sha(ROOT / "Cargo.lock"), grammar_sha256=sha(ROOT / "grammar/verum.ebnf"),
                runner_sha256=sha(Path(__file__)), reference_executable_sha256=sha(executable),
                reference_program_sha256=sha(source), commands=commands, reference_differences=delta,
                fixtures=fixtures, verum_runtime="NOT EXECUTED")
    (output / "plan.json").write_text(json.dumps(plan, indent=2) + "\n")
    require(not delta, f"{len(delta)} hand-authored oracle differences; inspect plan.json")
    print(json.dumps(dict(reference_cases=len(corpus["cases"]), reference_matches=sum(len(r["matches"]) for r in observations.values()),
                          fixtures=list(fixtures), plan=str(output / "plan.json"), verum_runtime="NOT EXECUTED")))


def compare(args):
    plan = json.loads(args.plan.read_text())
    fixture = plan["fixtures"][args.fixture]
    require(sha(Path(fixture["path"])) == fixture["sha256"], "fixture changed after preparation")
    records = parse_output(args.stdout.read_text(), fixture["cases"])
    delta = differences(records, fixture["cases"])
    result = dict(fixture=args.fixture, observed_cases=len(records),
                  observed_matches=sum(len(r["matches"]) for r in records.values()),
                  stdout_sha256=sha(args.stdout), plan_sha256=sha(args.plan), differences=delta,
                  verdict="FAIL" if delta else "PASS",
                  execution_scope="Output comparison only; attach pinned CLI exit/timeout/input receipt separately")
    args.result.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))
    return 1 if delta else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prep = commands.add_parser("prepare")
    prep.add_argument("--corpus", type=Path, default=CORPUS)
    prep.add_argument("--semver-source", type=Path, required=True)
    prep.add_argument("--semver-archive", type=Path, required=True)
    prep.add_argument("--rustc", type=Path, required=True)
    prep.add_argument("--output", type=Path, required=True)
    prep.add_argument("--api", choices=("compatibility", "explicit"), default="compatibility",
                      help="compatibility preserves the original parse-entry baseline; explicit selects the declared contracts")
    check = commands.add_parser("compare")
    check.add_argument("--plan", type=Path, required=True)
    check.add_argument("--fixture", required=True)
    check.add_argument("--stdout", type=Path, required=True)
    check.add_argument("--result", type=Path, required=True)
    args = parser.parse_args()
    try:
        return compare(args) if args.command == "compare" else prepare(args) or 0
    except (ValueError, KeyError, OSError, json.JSONDecodeError) as error:
        parser.exit(2, f"runner error: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
