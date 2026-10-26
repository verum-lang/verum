#!/usr/bin/env python3
"""Replay typed-program laws and reject altered proofs with the Lean kernel.

This harness does not produce or verify a native Verum certificate. Unexpected
tool, import, parser, resource or elaboration failures never count as controls.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent
MODULE = ROOT / "VerumExternalReplay/TypedPrograms.lean"
CONTROLS = ROOT / "VerumExternalReplay/TypedProgramControls.lean"
HEADER = """import VerumExternalReplay.TypedProgramControls
open TypedPrograms TypedProgramControls
set_option warningAsError true
"""


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(*args, timeout=180):
    return subprocess.run(args, cwd=ROOT, capture_output=True, text=True, timeout=timeout)


def require_success(result):
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    return result.stdout.strip()


def check(path, expected=None):
    result = run("lake", "env", "lean", "--json", str(path), timeout=60)
    # Lean's JSON diagnostics let us require the intended proof/type failure,
    # rather than accepting any nonzero process status as a successful control.
    diagnostics = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
    errors = [item for item in diagnostics if item["severity"] == "error"]
    if expected is None:
        if result.returncode or result.stderr or errors:
            raise RuntimeError(f"Positive proof failed: {path}\n{result.stdout}{result.stderr}")
    elif (result.returncode != 1 or result.stderr or not errors
          or not errors[0]["data"].startswith(expected)):
        raise RuntimeError(f"Control did not reach its intended rejection: {path}\n"
                           f"{result.stdout}{result.stderr}")
    return {"source_sha256": sha256(path), "accepted": result.returncode == 0,
            "diagnostics": diagnostics}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    sources = [Path(__file__).resolve(), MODULE, CONTROLS, ROOT / "lean-toolchain",
               ROOT / "lakefile.toml"]
    before = {str(path.relative_to(ROOT)): sha256(path) for path in sources}
    require_success(run("lake", "build", "VerumExternalReplay.TypedProgramControls"))
    positive = {path.stem: check(path) for path in (MODULE, CONTROLS)}
    equations = {
        "eager_failure": "unusedFailure.run target empty = erasedFailure.run target empty",
        "wrong_output": "compiled.run target (input 4) = some 9",
        "captured_register": "captured.run target (input 3) = "
                             "(increment.splice continuation).run target (input 3)",
        "changed_literal": "(returnLiteral.translate changedLiteral expansion).run target empty = "
                           "returnLiteral.run source empty",
        "inexact_primitive_value": "(shared.translate (fun x => x) inaccurateExpansion).run "
                                   "target (input 4) = shared.run source (input 4)",
        "inexact_primitive_failure": "(shared.translate (fun x => x) inaccurateExpansion).run "
                                     "target (input 0) = shared.run source (input 0)",
        "noninjective_result_reflection": "(some 1 : Option Nat).map (fun _ => 0) = some 0 ↔ "
                                          "(some 1 : Option Nat) = some 2",
    }
    negatives = {}
    with tempfile.TemporaryDirectory(prefix="verum-typed-programs-") as directory:
        directory = Path(directory)
        for name, equation in equations.items():
            path = directory / f"{name}.lean"
            path.write_text(HEADER + f"example : {equation} := by rfl\n")
            negatives[name] = check(path, "Tactic `rfl` failed:")
        path = directory / "wrong_register_sort.lean"
        path.write_text(HEADER + "example : Ref [Kind.flag, Kind.number] Kind.number := .here\n")
        negatives[path.stem] = check(path, "Type mismatch")
        mutations = {
            "literal_evidence_removed": ("| literal l => exact literal_exact l", "| literal _ => rfl"),
            "continuation_evidence_removed": ("rw [← Env.map_cons, ih]", "rfl"),
        }
        source = MODULE.read_text()
        for name, (old, new) in mutations.items():
            if source.count(old) != 1:
                raise RuntimeError(f"Mutation target is no longer unique: {name}")
            path = directory / f"{name}.lean"
            path.write_text(source.replace(old, new))
            negatives[name] = check(path, "Tactic `rfl` failed:")
    after = {str(path.relative_to(ROOT)): sha256(path) for path in sources}
    if before != after:
        raise RuntimeError("Proof sources changed during verification")
    lean_prefix = Path(require_success(run("lake", "env", "lean", "--print-prefix")))
    checker_files = [lean_prefix / "bin/lean"]
    checker_files += sorted(path for path in (lean_prefix / "lib/lean").iterdir()
                            if path.is_file() and path.suffix in {".dylib", ".so", ".dll"})
    report = {
        "scope": "Direct Lean typed finite program theorem; native indexed syntax/lowering remains open",
        "lean_version": require_success(run("lake", "env", "lean", "--version")),
        "checker_sha256": {str(path.relative_to(lean_prefix)): sha256(path) for path in checker_files},
        "source_sha256": before, "positive": positive, "negative": negatives,
    }
    args.report.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    print(f"Typed program laws verified; {len(negatives)} intended proof/type rejections verified.")


if __name__ == "__main__":
    main()
